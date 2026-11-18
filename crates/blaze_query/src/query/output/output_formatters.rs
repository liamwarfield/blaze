//! Port of `query2/query/output/OutputFormatters.java`.

use crate::query::output::build_output_formatter::BuildOutputFormatter;
use crate::query::output::label_output_formatter::LabelOutputFormatter;
use crate::query::output::location_output_formatter::LocationOutputFormatter;
use crate::query::output::output_formatter::OutputFormatter;
use crate::query::output::package_output_formatter::PackageOutputFormatter;
use crate::query::output::proto_output_formatter::{ProtoKind, ProtoOutputFormatter};

/// A formatter of the default list that is not ported yet (`minrank`, `maxrank`,
/// `graph`, `xml`): it keeps the name in the list (for `getFormatter` and the error message) and
/// fails loudly when used.
struct UnportedFormatter {
    name: &'static str,
    streamed: bool,
}

impl OutputFormatter for UnportedFormatter {
    fn get_name(&self) -> &'static str {
        self.name
    }

    fn as_streamed_formatter(
        &self,
    ) -> Option<&dyn crate::query::output::output_formatter::StreamedFormatter> {
        if self.streamed {
            blaze_util::unported!("OutputFormatter", "--output={}", self.name)
        }
        None
    }

    fn output_digraph(&self) {
        blaze_util::unported!("OutputFormatter", "--output={}", self.name)
    }
}

/// Same as `getDefaultFormatters()`.
// Java: com/google/devtools/build/lib/query2/query/output/OutputFormatters.java:31 OutputFormatters.getDefaultFormatters
pub fn get_default_formatters() -> Vec<Box<dyn OutputFormatter>> {
    vec![
        Box::new(LabelOutputFormatter::new(false)),
        Box::new(LabelOutputFormatter::new(true)),
        Box::new(BuildOutputFormatter),
        Box::new(UnportedFormatter {
            name: "minrank",
            streamed: false,
        }),
        Box::new(UnportedFormatter {
            name: "maxrank",
            streamed: false,
        }),
        Box::new(PackageOutputFormatter),
        Box::new(LocationOutputFormatter::default()),
        Box::new(UnportedFormatter {
            name: "graph",
            streamed: false,
        }),
        Box::new(UnportedFormatter {
            name: "xml",
            streamed: true,
        }),
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

/// PORT: `instanceof StreamedFormatter` is a static property of each formatter class; for the
/// unported ones it is taken from `UnportedFormatter.streamed` without calling into them.
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
    match formatter.get_name() {
        // Java: XmlOutputFormatter extends AbstractUnorderedFormatter.
        "xml" => true,
        "minrank" | "maxrank" | "graph" => false,
        _ => formatter.as_streamed_formatter().is_some(),
    }
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
