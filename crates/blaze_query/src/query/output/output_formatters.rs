//! Port of `query2/query/output/OutputFormatters.java`.

use crate::query::output::build_output_formatter::BuildOutputFormatter;
use crate::query::output::graph_output_formatter::GraphOutputFormatter;
use crate::query::output::label_output_formatter::LabelOutputFormatter;
use crate::query::output::location_output_formatter::LocationOutputFormatter;
use crate::query::output::maxrank_output_formatter::MaxrankOutputFormatter;
use crate::query::output::minrank_output_formatter::MinrankOutputFormatter;
use crate::query::output::output_formatter::OutputFormatter;
use crate::query::output::package_output_formatter::PackageOutputFormatter;
use crate::query::output::proto_output_formatter::{ProtoKind, ProtoOutputFormatter};
use crate::query::output::xml_output_formatter::XmlOutputFormatter;

/// Same as `getDefaultFormatters()`.
// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatters.java:31 OutputFormatters.getDefaultFormatters
pub fn get_default_formatters() -> Vec<Box<dyn OutputFormatter>> {
    vec![
        Box::new(LabelOutputFormatter::new(false)),
        Box::new(LabelOutputFormatter::new(true)),
        Box::new(BuildOutputFormatter),
        Box::new(MinrankOutputFormatter),
        Box::new(MaxrankOutputFormatter),
        Box::new(PackageOutputFormatter),
        Box::new(LocationOutputFormatter::default()),
        Box::new(GraphOutputFormatter),
        Box::new(XmlOutputFormatter::default()),
        Box::new(ProtoOutputFormatter::new(ProtoKind::Proto)),
        Box::new(ProtoOutputFormatter::new(ProtoKind::StreamedJsonProto)),
        Box::new(ProtoOutputFormatter::new(ProtoKind::StreamedProto)),
    ]
}

// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatters.java:47 OutputFormatters.formatterNames
pub fn formatter_names(formatters: &[Box<dyn OutputFormatter>]) -> String {
    formatters
        .iter()
        .map(|f| f.get_name())
        .collect::<Vec<_>>()
        .join(", ")
}

// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatters.java:51 OutputFormatters.streamingFormatterNames
pub fn streaming_formatter_names(formatters: &[Box<dyn OutputFormatter>]) -> String {
    formatters
        .iter()
        .filter(|f| is_streamed_formatter(&***f))
        .map(|f| f.get_name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Java `formatter instanceof StreamedFormatter`.
pub fn is_streamed_formatter(formatter: &dyn OutputFormatter) -> bool {
    formatter.as_streamed_formatter().is_some()
}

/// Returns the output formatter for the specified command-line options.
// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatters.java:63 OutputFormatters.getFormatter
pub fn get_formatter<'a>(
    formatters: &'a [Box<dyn OutputFormatter>],
    type_: &str,
) -> Option<&'a dyn OutputFormatter> {
    for formatter in formatters {
        if formatter.get_name() == type_ {
            return Some(&**formatter);
        }
    }
    None
}
