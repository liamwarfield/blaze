//! Port of `query2/query/output/BuildOutputFormatter.java`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::attribute::raw_attribute_mapper::RawAttributeMapper;
use blaze_packages::attribute::{AbstractAttributeMapper, Attribute};
use blaze_packages::target::{LabelPrinter, Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::AttrValue;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, SynchronizedDelegatingOutputFormatterCallback,
    ThreadSafeOutputFormatterCallback,
};
use blaze_starlark::error::StackEntry;
use blaze_vfs::DigestHashFunction;

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::output::output_formatter::{
    OutputFormatter, OutputStream, StreamedFormatter, TextWriter,
};
use crate::query::output::possible_attribute_values;
use crate::query::output::query_options::get_line_terminator;

/// Java `Printer.appendQuoted` / `escapeCharacter` (Starlark string repr).
// Java: net/starlark/java/eval/Printer.java:225 Printer.appendQuoted
fn append_quoted(buf: &mut String, s: &str) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\r' => buf.push_str("\\r"),
            '\n' => buf.push_str("\\n"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 32 => buf.push_str(&format!("\\x{:02x}", c as u32)),
            c => buf.push(c), // no need to support UTF-8
        }
    }
    buf.push('"');
}

/// The values `outputRawAttrValue` prints (Java objects after its License/TriState rewrites).
enum ReprValue {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    Label(Label),
    List(Vec<ReprValue>),
    Dict(Vec<(ReprValue, ReprValue)>),
}

impl ReprValue {
    fn of(value: &AttrValue) -> ReprValue {
        match value {
            AttrValue::Int(i) => ReprValue::Int(*i as i64),
            AttrValue::String(s) => ReprValue::Str(s.clone()),
            AttrValue::Bool(b) => ReprValue::Bool(*b),
            AttrValue::Label(l) => ReprValue::Label(l.clone()),
            AttrValue::StringList(l) => {
                ReprValue::List(l.iter().cloned().map(ReprValue::Str).collect())
            }
            AttrValue::StringSet(l) => {
                ReprValue::List(l.iter().cloned().map(ReprValue::Str).collect())
            }
            AttrValue::IntList(l) => {
                ReprValue::List(l.iter().map(|i| ReprValue::Int(*i as i64)).collect())
            }
            AttrValue::LabelList(l) => {
                ReprValue::List(l.iter().cloned().map(ReprValue::Label).collect())
            }
            AttrValue::StringDict(d) => ReprValue::Dict(
                d.iter()
                    .map(|(k, v)| (ReprValue::Str(k.clone()), ReprValue::Str(v.clone())))
                    .collect(),
            ),
            AttrValue::StringListDict(d) => ReprValue::Dict(
                d.iter()
                    .map(|(k, v)| {
                        (
                            ReprValue::Str(k.clone()),
                            ReprValue::List(v.iter().cloned().map(ReprValue::Str).collect()),
                        )
                    })
                    .collect(),
            ),
            AttrValue::LabelDictUnary(d) => ReprValue::Dict(
                d.iter()
                    .map(|(k, v)| (ReprValue::Str(k.clone()), ReprValue::Label(v.clone())))
                    .collect(),
            ),
            AttrValue::LabelKeyedStringDict(d) => ReprValue::Dict(
                d.iter()
                    .map(|(k, v)| (ReprValue::Label(k.clone()), ReprValue::Str(v.clone())))
                    .collect(),
            ),
            AttrValue::LabelListDict(d) => ReprValue::Dict(
                d.iter()
                    .map(|(k, v)| {
                        (
                            ReprValue::Str(k.clone()),
                            ReprValue::List(v.iter().cloned().map(ReprValue::Label).collect()),
                        )
                    })
                    .collect(),
            ),
            // if (value instanceof License): the lower-cased license type names.
            AttrValue::License(license) => ReprValue::List(
                license
                    .get_license_types()
                    .iter()
                    .map(|t| ReprValue::Str(t.name().to_ascii_lowercase()))
                    .collect(),
            ),
            // else if (value instanceof TriState triState): value = triState.toInt()
            AttrValue::TriState(t) => ReprValue::Int(t.to_int() as i64),
            other => blaze_util::unported!("Printer.repr", "{other:?}"),
        }
    }

    /// Java `Printer.repr` with `outputRawAttrValue`'s override printing labels with the label
    /// printer.
    // Java: net/starlark/java/eval/Printer.java:117 Printer.repr
    fn repr(&self, buf: &mut String, label_printer: &LabelPrinter) {
        match self {
            ReprValue::None => buf.push_str("None"),
            ReprValue::Bool(b) => buf.push_str(if *b { "True" } else { "False" }),
            ReprValue::Int(i) => buf.push_str(&i.to_string()),
            ReprValue::Str(s) => append_quoted(buf, s),
            ReprValue::Label(l) => append_quoted(buf, &label_printer.to_string(l)),
            ReprValue::List(list) => {
                buf.push('[');
                for (i, x) in list.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    x.repr(buf, label_printer);
                }
                buf.push(']');
            }
            ReprValue::Dict(entries) => {
                buf.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    k.repr(buf, label_printer);
                    buf.push_str(": ");
                    v.repr(buf, label_printer);
                }
                buf.push('}');
            }
        }
    }
}

/// Outputs the targets as BUILD rules (Java `BuildOutputFormatter.TargetOutputter`).
// Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:62 BuildOutputFormatter.TargetOutputter
pub struct TargetOutputter {
    pub writer: TextWriter,
    preserve_select: Box<dyn Fn(&Rule, &Attribute) -> bool>,
    line_term: &'static str,
    label_printer: LabelPrinter,
    printed: RefCell<HashSet<Label>>,
}

/// Java `BuildOutputFormatter.AttributeReader`: the possible values of an attribute of a rule.
// Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:56 BuildOutputFormatter.AttributeReader
pub type AttributeReader<'a> = &'a dyn Fn(&Rule, &Attribute) -> Vec<Option<AttrValue>>;

impl TargetOutputter {
    /// - `writer`: where to write output
    /// - `preserve_select`: a predicate that determines if the given attribute on the given rule
    ///   is a `select` that should be output as-is (without figuring out which branch it takes).
    ///   This is useful for implementations that can't evaluate `select`.
    /// - `line_term`: character to add to the end of each line
    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:69 TargetOutputter.<init>
    pub fn new(
        writer: TextWriter,
        preserve_select: Box<dyn Fn(&Rule, &Attribute) -> bool>,
        line_term: &'static str,
        label_printer: LabelPrinter,
    ) -> Self {
        TargetOutputter {
            writer,
            preserve_select,
            line_term,
            label_printer,
            printed: RefCell::new(HashSet::new()),
        }
    }

    /// Outputs a given target in BUILD-style syntax.
    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:80 TargetOutputter.output
    pub fn output(&self, target: &Target, attr_reader: AttributeReader<'_>) {
        let rule = match target.get_associated_rule() {
            Some(rule) => rule,
            None => return,
        };
        if self.printed.borrow().contains(rule.get_label()) {
            return;
        }
        self.output_rule(rule, attr_reader);
        self.printed.borrow_mut().insert(rule.get_label().clone());
    }

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:91 TargetOutputter.outputRule
    fn output_rule(&self, rule: &Rule, attr_reader: AttributeReader<'_>) {
        let w = &self.writer;
        let lt = self.line_term;
        w.append("# ")
            .append(&rule.get_location().to_string())
            .append(lt);
        w.append(rule.get_rule_class()).append("(").append(lt);
        w.append("  name = \"")
            .append(rule.get_name())
            .append("\",")
            .append(lt);

        for attr in rule.get_attributes() {
            // Ignore the "name" attribute here, as we already print it above.
            // This is not strictly necessary, but convenient for testing.
            if attr.get_name() == "name" {
                continue;
            }
            if (self.preserve_select)(rule, attr) {
                w.append("  ")
                    .append(&attr.get_public_name())
                    .append(" = ")
                    .append(&self.reconstruct_select(rule, attr))
                    .append(",")
                    .append(lt);
                continue;
            }
            if !rule.is_attribute_value_explicitly_specified(attr.get_name()) {
                continue; // Don't print default values.
            }
            let mut values = attr_reader(rule, attr);
            if values.len() != 1 {
                // Computed defaults that depend on configurable attributes can have multiple
                // values.
                continue;
            }
            let value = values.pop().unwrap();
            w.append("  ")
                .append(&attr.get_public_name())
                .append(" = ")
                .append(&self.output_raw_attr_value(value.as_ref()))
                .append(",")
                .append(lt);
        }
        w.append(")").append(lt);

        // Display the instantiation stack, if any.
        self.append_stack(
            &format!(
                "# Rule {} instantiated at (most recent call last):",
                rule.get_name()
            ),
            &rule.reconstruct_call_stack(),
        );

        // Display the stack of the rule class definition, if any.
        self.append_stack(
            &format!(
                "# Rule {} defined at (most recent call last):",
                rule.get_rule_class()
            ),
            rule.get_rule_class_object().get_call_stack(),
        );

        // TODO(adonovan): also list inputs and outputs of the rule.

        w.append(lt);
    }

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:148 TargetOutputter.appendStack
    fn append_stack(&self, title: &str, stack: &[StackEntry]) {
        let location_string = |fr: &StackEntry| match &fr.location {
            Some(l) => l.to_string(),
            None => "<builtin>".to_owned(),
        };
        // For readability, ensure columns line up.
        let mut max_loc_len = 0;
        for fr in stack {
            // Java String.length: chars (of an internal string, one per byte).
            max_loc_len = max_loc_len.max(location_string(fr).chars().count());
        }
        if max_loc_len > 0 {
            self.writer.append(title).append(self.line_term);
            for fr in stack {
                let loc = location_string(fr); // TODO(b/151151653): display root-relative
                self.writer.append("#   ").append(&loc);
                for _ in loc.chars().count()..max_loc_len {
                    self.writer.append(" ");
                }
                self.writer
                    .append(" in ")
                    .append(&fr.name)
                    .append(self.line_term);
            }
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:170 TargetOutputter.outputRawAttrValue
    fn output_raw_attr_value(&self, value: Option<&AttrValue>) -> String {
        let repr_value = match value {
            None => ReprValue::None,
            Some(v) => ReprValue::of(v),
        };
        let mut buf = String::new();
        repr_value.repr(&mut buf, &self.label_printer);
        buf
    }

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:188 TargetOutputter.reconstructSelect
    fn reconstruct_select(&self, rule: &Rule, attr: &Attribute) -> String {
        let mut selectors: Vec<String> = Vec::new();
        let attribute_map = RawAttributeMapper::of(rule);
        let selector_list = match attribute_map.get_raw_attribute_value_of(rule, attr) {
            Some(AttrValue::SelectorList(selector_list)) => selector_list,
            other => panic!("ClassCastException: not a SelectorList: {other:?}"),
        };
        for selector in selector_list.get_selectors() {
            if selector.is_unconditional() {
                selectors.push(
                    self.output_raw_attr_value(Some(
                        selector
                            .get_default()
                            .expect("NullPointerException: unconditional select default"),
                    )),
                );
            } else {
                let mut entries: Vec<(ReprValue, ReprValue)> = Vec::new();
                for (label, value) in selector.map_copy() {
                    entries.push((
                        ReprValue::Label(label),
                        match &value {
                            None => ReprValue::None,
                            Some(v) => ReprValue::of(v),
                        },
                    ));
                }
                let mut buf = String::new();
                ReprValue::Dict(entries).repr(&mut buf, &self.label_printer);
                selectors.push(format!("select({buf})"));
            }
        }
        selectors.join(" + ")
    }
}

/// An output formatter that prints the result as if it were in BUILD language syntax.
// Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:46 BuildOutputFormatter
pub struct BuildOutputFormatter;

impl OutputFormatter for BuildOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:268 BuildOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "build"
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for BuildOutputFormatter {
    fn set_options(
        &self,
        _options: &QueryOptions,
        _aspect_resolver: Arc<dyn AspectResolver>,
        _hash_function: DigestHashFunction,
    ) {
    }

    fn set_event_handler(&self, _event_handler: Option<Arc<dyn EventHandler>>) {}

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:225 BuildOutputFormatter.createStreamCallback
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

    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:218 BuildOutputFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        Box::new(BuildOutputFormatterCallback {
            target_outputter: TargetOutputter::new(
                TextWriter::new(out),
                Box::new(|rule: &Rule, attr: &Attribute| {
                    AbstractAttributeMapper::is_configurable_in_with(
                        rule,
                        attr.get_name(),
                        /* include_computed_defaults= */ false,
                    )
                }),
                get_line_terminator(options),
                label_printer.clone(),
            ),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

// Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:232 BuildOutputFormatter.BuildOutputFormatterCallback
struct BuildOutputFormatterCallback {
    target_outputter: TargetOutputter,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for BuildOutputFormatterCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for BuildOutputFormatterCallback {
    // Java: com/google/devtools/build/lib/query2/query/output/BuildOutputFormatter.java:253 BuildOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            self.target_outputter.output(
                target,
                // Multiple possible values are ignored by the outputter.
                &|rule: &Rule, attr: &Attribute| {
                    possible_attribute_values::for_rule_and_attribute(
                        rule, attr, /* may_treat_multiple_as_none= */ true,
                    )
                },
            );
        }
        Ok(())
    }

    fn close(&self, _fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.target_outputter.writer.flush()?;
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
