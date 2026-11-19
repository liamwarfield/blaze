//! Port of `query2/query/output/PackageOutputFormatter.java`.

use std::cell::RefCell;
use std::collections::BTreeSet;
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

/// An output formatter that prints the set of packages of the resulting target set in
/// lexicographical order without duplicates.
// Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:32 PackageOutputFormatter
pub struct PackageOutputFormatter;

impl OutputFormatter for PackageOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:35 PackageOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "package"
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for PackageOutputFormatter {
    fn set_options(
        &self,
        _options: &QueryOptions,
        _aspect_resolver: Arc<dyn AspectResolver>,
        _hash_function: DigestHashFunction,
    ) {
    }

    fn set_event_handler(&self, _event_handler: Option<Arc<dyn EventHandler>>) {}

    // Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:72 PackageOutputFormatter.createStreamCallback
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

    // Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:40 PackageOutputFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        Box::new(PackageCallback {
            writer: TextWriter::new(out),
            package_names: RefCell::new(BTreeSet::new()),
            line_term: get_line_terminator(options),
            label_printer: label_printer.clone(),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

struct PackageCallback {
    writer: TextWriter,
    /// Java `Sets.newTreeSet()` of internal strings: `String.compareTo` on Latin-1 characters
    /// (one UTF-16 unit each) is `str` order.
    package_names: RefCell<BTreeSet<String>>,
    line_term: &'static str,
    label_printer: LabelPrinter,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for PackageCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for PackageCallback {
    // Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:46 PackageOutputFormatter.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            let mut package_label = self
                .label_printer
                .to_string_package(target.get_label().get_package_identifier());
            if let Some(rest) = package_label.strip_prefix("//") {
                package_label = rest.to_owned();
            }
            self.package_names.borrow_mut().insert(package_label);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/PackageOutputFormatter.java:59 PackageOutputFormatter.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if !fail_fast {
            for package_name in self.package_names.borrow().iter() {
                self.writer.append(package_name).append(self.line_term);
            }
        }
        self.writer.flush()?;
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
