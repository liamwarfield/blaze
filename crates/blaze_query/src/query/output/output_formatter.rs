//! Ports of `OutputFormatter.java`, `StreamedFormatter.java`, `AbstractUnorderedFormatter.java`
//! and `TextOutputFormatterCallback.java`.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::callback::{OutputFormatterError, process_all_targets};
use blaze_query_engine::{
    OutputFormatterCallback, QueryException, QueryExpression, ThreadSafeOutputFormatterCallback,
};
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_vfs::DigestHashFunction;

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::blaze_query_environment::{TargetDigraph, TargetNode};
use crate::query::output::format_utils;

/// Java `OutputStream`, shared between the command and the callbacks it creates.
pub type OutputStream = Rc<RefCell<dyn Write>>;

/// Write the bytes to the stream, mapping the error to the formatter error type.
pub fn write_all(out: &OutputStream, bytes: &[u8]) -> std::io::Result<()> {
    out.borrow_mut().write_all(bytes)
}

/// Interface for classes which order, format and print the result of a Blaze graph query.
// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatter.java:31 OutputFormatter
pub trait OutputFormatter {
    /// Returns the user-visible name of the output formatter.
    // Java: com/google/devtools/build/lib/query2/query/output/OutputFormatter.java:37 OutputFormatter.getName
    fn get_name(&self) -> &'static str;

    /// Returns true iff the output formatter's output can be buffered.
    // Java: com/google/devtools/build/lib/query2/query/output/OutputFormatter.java:40 OutputFormatter.canBeBuffered
    fn can_be_buffered(&self) -> bool {
        true
    }

    /// Throws an exception if the specified expression cannot be used with this formatter.
    // Java: com/google/devtools/build/lib/query2/query/output/OutputFormatter.java:47 OutputFormatter.verifyCompatible
    fn verify_compatible(
        &self,
        _env: &dyn AbstractBlazeQueryEnvironment,
        _expr: &QueryExpression,
    ) -> Result<(), QueryException> {
        Ok(())
    }

    /// Format the result (a set of target nodes implicitly ordered according to the graph
    /// maintained by the QueryEnvironment), and print it to "out".
    ///
    /// The default is `AbstractUnorderedFormatter.output`, for the formatters that are
    /// `StreamedFormatter`s; the others override it.
    // Java: com/google/devtools/build/lib/query2/query/output/OutputFormatter.java:54 OutputFormatter.output
    // Java: com/google/devtools/build/lib/query2/query/output/AbstractUnorderedFormatter.java:43 AbstractUnorderedFormatter.output
    fn output(
        &self,
        options: &QueryOptions,
        result: &TargetDigraph,
        out: OutputStream,
        aspect_resolver: Arc<dyn AspectResolver>,
        event_handler: Option<Arc<dyn EventHandler>>,
        hash_function: DigestHashFunction,
        label_printer: &LabelPrinter,
    ) -> Result<(), OutputFormatterError> {
        let streamed = self
            .as_streamed_formatter()
            .unwrap_or_else(|| panic!("AbstractMethodError: {}.output(Digraph)", self.get_name()));
        streamed.set_options(options, aspect_resolver, hash_function);
        streamed.set_event_handler(event_handler);
        process_all_targets(
            &*streamed.create_post_facto_stream_callback(out, options, label_printer),
            &get_ordered_targets(result, options),
        )
    }

    /// Java `formatter instanceof StreamedFormatter`.
    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        None
    }
}

/// An `OutputFormatter` that can output results as they are computed.
// Java: com/google/devtools/build/lib/query2/query/output/StreamedFormatter.java:28 StreamedFormatter
pub trait StreamedFormatter {
    /// Specifies options to be used by subsequent calls to `create_stream_callback`.
    // Java: com/google/devtools/build/lib/query2/query/output/StreamedFormatter.java:30 StreamedFormatter.setOptions
    fn set_options(
        &self,
        options: &QueryOptions,
        aspect_resolver: Arc<dyn AspectResolver>,
        hash_function: DigestHashFunction,
    );

    /// Sets `EventHandler` for outputting messages.
    // Java: com/google/devtools/build/lib/query2/query/output/StreamedFormatter.java:34 StreamedFormatter.setEventHandler
    fn set_event_handler(&self, event_handler: Option<Arc<dyn EventHandler>>);

    /// Returns a `ThreadSafeOutputFormatterCallback` whose `process` outputs formatted targets
    /// to the given `out`.
    ///
    /// Takes any options specified via the most recent call to `set_options` into
    /// consideration.
    ///
    /// Intended to be use for streaming out during evaluation of a query.
    // Java: com/google/devtools/build/lib/query2/query/output/StreamedFormatter.java:44 StreamedFormatter.createStreamCallback
    fn create_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        env: &dyn AbstractBlazeQueryEnvironment,
    ) -> Box<dyn ThreadSafeOutputFormatterCallback<Target>>;

    /// Same as `create_stream_callback`, but intended to be used for outputting the
    /// already-computed result of a query.
    // Java: com/google/devtools/build/lib/query2/query/output/StreamedFormatter.java:52 StreamedFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>>;
}

/// The targets of `result` in the order `--order_output` asks for.
// Java: com/google/devtools/build/lib/query2/query/output/AbstractUnorderedFormatter.java:59 AbstractUnorderedFormatter.getOrderedTargets
pub fn get_ordered_targets(result: &TargetDigraph, options: &QueryOptions) -> Vec<Target> {
    if options.order_output == OrderOutput::FULL {
        // Get targets in total order, the difference here from topological ordering is the
        // sorting of nodes before post-order visitation (which ensures determinism at a time
        // cost).
        let ordering = |a: &TargetNode, b: &TargetNode| format_utils::target_ordering(&a.0, &b.0);
        result
            .get_topological_order_by(&ordering)
            .into_iter()
            .map(|n| result.get_label(n).0.clone())
            .collect()
    } else if options.order_output == OrderOutput::DEPS {
        // Get targets in topological order.
        result
            .get_topological_order()
            .into_iter()
            .map(|n| result.get_label(n).0.clone())
            .collect()
    } else {
        result
            .get_labels()
            .into_iter()
            .map(|t| t.0.clone())
            .collect()
    }
}

/// Java `TextOutputFormatterCallback`'s writer: a `BufferedWriter(OutputStreamWriter(out))`
/// flushed by `close`.
///
/// The `OutputStreamWriter` has the default charset, ISO-8859-1 on Bazel's server
/// (`-Dfile.encoding=ISO-8859-1`): an internal string leaves as its raw bytes, a char above
/// U+00FF as `?`.
// Java: com/google/devtools/build/lib/query2/query/output/TextOutputFormatterCallback.java:26 TextOutputFormatterCallback
pub struct TextWriter {
    out: OutputStream,
    buffer: RefCell<Vec<u8>>,
}

impl TextWriter {
    // Java: com/google/devtools/build/lib/query2/query/output/TextOutputFormatterCallback.java:30 TextOutputFormatterCallback.<init>
    pub fn new(out: OutputStream) -> Self {
        TextWriter {
            out,
            buffer: RefCell::new(Vec::new()),
        }
    }

    /// Java `writer.append(s)`.
    pub fn append(&self, s: &str) -> &Self {
        self.buffer
            .borrow_mut()
            .extend_from_slice(&latin1_encode_lossy(s));
        self
    }

    /// Java `writer.flush()` (`TextOutputFormatterCallback.close`).
    // Java: com/google/devtools/build/lib/query2/query/output/TextOutputFormatterCallback.java:36 TextOutputFormatterCallback.close
    pub fn flush(&self) -> std::io::Result<()> {
        let bytes = std::mem::take(&mut *self.buffer.borrow_mut());
        let mut out = self.out.borrow_mut();
        out.write_all(&bytes)?;
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The oracle's `query //:all --output=label_kind` on the byte-strings workspace
    // (~/.cache/blaze/work/bytes-out/query_label_kind.txt) prints `//:fg_é` as its raw bytes
    // 66 67 5F C3 A9, and `--output=build` prints the byte FF of `"é\377"` as FF: the writer
    // encodes the internal string's chars as Latin-1.
    #[test]
    fn text_writer_writes_internal_strings_as_raw_bytes() {
        let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
        let out: OutputStream = buffer.clone();
        let writer = TextWriter::new(out);
        writer
            .append("//:fg_\u{c3}\u{a9}")
            .append(" \u{c3}\u{a9}\u{f0}\u{9f}\u{90}\u{b7}\u{ff}\n");
        writer.flush().unwrap();
        assert_eq!(
            &*buffer.borrow(),
            b"//:fg_\xc3\xa9 \xc3\xa9\xf0\x9f\x90\xb7\xff\n"
        );
    }

    // A Unicode char above U+00FF (only a Java literal can hold one) becomes '?' in the
    // default-charset writer.
    #[test]
    fn text_writer_replaces_chars_above_latin1() {
        let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
        let out: OutputStream = buffer.clone();
        let writer = TextWriter::new(out);
        writer.append("a\u{1f437}b");
        writer.flush().unwrap();
        assert_eq!(&*buffer.borrow(), b"a?b");
    }
}
