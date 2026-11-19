//! Port of `query2/query/output/XmlOutputFormatter.java`, with the part of the JDK's DOM and
//! `Transformer` (Xalan) serialization the formatter's output depends on.

use std::cell::RefCell;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::attribute::AttributeInfoProvider;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::target::{LabelPrinter, RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, InterruptedException, OutputFormatterCallback, QueryError,
    SynchronizedDelegatingOutputFormatterCallback, ThreadSafeOutputFormatterCallback,
};
use blaze_vfs::DigestHashFunction;
use indexmap::IndexSet;

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::output::format_utils;
use crate::query::output::output_formatter::{OutputFormatter, OutputStream, StreamedFormatter};
use crate::query::output::possible_attribute_values;

/// An output formatter that prints the result as XML.
// Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:60 XmlOutputFormatter
#[derive(Default)]
pub struct XmlOutputFormatter {
    state: RefCell<Option<XmlState>>,
}

/// The fields `setOptions` sets.
#[derive(Clone)]
struct XmlState {
    aspect_resolver: Arc<dyn AspectResolver>,
    dependency_filter: DependencyFilter,
    package_group_includes_double_slash: bool,
    relative_locations: bool,
    xml_show_default_values: bool,
    xml_line_numbers: bool,
}

impl OutputFormatter for XmlOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:69 XmlOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "xml"
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for XmlOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:81 XmlOutputFormatter.setOptions
    fn set_options(
        &self,
        options: &QueryOptions,
        aspect_resolver: Arc<dyn AspectResolver>,
        _hash_function: DigestHashFunction,
    ) {
        *self.state.borrow_mut() = Some(XmlState {
            aspect_resolver,
            dependency_filter: format_utils::get_dependency_filter(options),
            package_group_includes_double_slash: options
                .incompatible_package_group_includes_double_slash,
            relative_locations: options.relative_locations,
            xml_show_default_values: options.xml_show_default_values,
            xml_line_numbers: options.xml_line_numbers,
        });
    }

    fn set_event_handler(&self, _event_handler: Option<Arc<dyn EventHandler>>) {}

    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:73 XmlOutputFormatter.createStreamCallback
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

    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:94 XmlOutputFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        _options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        let state = self
            .state
            .borrow()
            .clone()
            .expect("NullPointerException: setOptions was not called");
        Box::new(XmlCallback {
            out,
            state,
            label_printer: label_printer.clone(),
            query_elem: RefCell::new(None),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

struct XmlCallback {
    out: OutputStream,
    state: XmlState,
    label_printer: LabelPrinter,
    /// `doc` and `queryElem` (the document has the single root element).
    query_elem: RefCell<Option<Element>>,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for XmlCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for XmlCallback {
    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:102 XmlOutputFormatter.createPostFactoStreamCallback.start
    fn start(&self) -> Result<(), blaze_query_engine::callback::IoException> {
        // doc.setXmlVersion("1.1") is written by the serializer below.
        let mut query_elem = Element::new("query");
        query_elem.set_attribute("version", "2");
        *self.query_elem.borrow_mut() = Some(query_elem);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:117 XmlOutputFormatter.createPostFactoStreamCallback.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            let elem = self.create_target_element(target)?;
            self.query_elem
                .borrow_mut()
                .as_mut()
                .expect("start() was not called")
                .append_child(elem);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:124 XmlOutputFormatter.createPostFactoStreamCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if !fail_fast {
            let doc = self.query_elem.borrow();
            let text = serialize_document(doc.as_ref().expect("start() was not called"));
            // StreamResult(out): the transformer writes the UTF-8 bytes and does not close.
            self.out
                .borrow_mut()
                .write_all(text.as_bytes())
                .map_err(|e| OutputFormatterError::Io(e.into()))?;
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

impl XmlCallback {
    /// Creates and returns a new DOM tree for the specified build target.
    ///
    /// XML structure: - element tag is &lt;source-file>, &lt;generated-file> or &lt;rule
    /// class="cc_library">, following the terminology of `Target.getTargetKind()`. - 'name'
    /// attribute is target's label. - 'location' attribute is consistent with output of --output
    /// location. - rule attributes are represented in the DOM structure.
    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:148 XmlOutputFormatter.createTargetElement
    fn create_target_element(&self, target: &Target) -> Result<Element, InterruptedException> {
        let state = &self.state;
        let label_printer = &self.label_printer;
        let mut elem;
        match target {
            Target::Rule(rule) => {
                elem = Element::new("rule");
                elem.set_attribute("class", &rule.get_rule_class());
                for attr in rule.get_attributes() {
                    if rule.is_attribute_value_explicitly_specified_for(attr)
                        || state.xml_show_default_values
                    {
                        // TODO(b/162524370): mayTreatMultipleAsNone should be true for types that
                        //  drop multiple values.
                        let values = possible_attribute_values::for_rule_and_attribute(
                            rule, attr, /* may_treat_multiple_as_none= */ false,
                        );
                        let mut attr_elem =
                            create_value_element(attr.get_type(), &values, label_printer);
                        attr_elem.set_attribute("name", attr.get_name());
                        elem.append_child(attr_elem);
                    }
                }

                // Include explicit elements for all direct inputs and outputs of a rule; this goes
                // beyond what is available from the attributes above, since it may also
                // (depending on options) include implicit outputs, exec-configuration outputs,
                // and default values.
                for label in rule.get_sorted_labels(&state.dependency_filter) {
                    let mut input_elem = Element::new("rule-input");
                    input_elem.set_attribute("name", &label_printer.to_string(&label));
                    elem.append_child(input_elem);
                }

                let aspect_dependencies = state
                    .aspect_resolver
                    .compute_aspect_dependencies(target, &state.dependency_filter)?;
                let mut distinct: IndexSet<Label> = IndexSet::new();
                for (_, attribute_labels) in &aspect_dependencies {
                    for (_, labels) in attribute_labels.values() {
                        for label in labels {
                            distinct.insert(label.clone());
                        }
                    }
                }
                for label in distinct {
                    let mut input_elem = Element::new("rule-input");
                    input_elem.set_attribute("name", &label_printer.to_string(&label));
                    elem.append_child(input_elem);
                }

                for output_file in rule.get_output_files() {
                    let mut output_elem = Element::new("rule-output");
                    output_elem
                        .set_attribute("name", &label_printer.to_string(output_file.get_label()));
                    elem.append_child(output_elem);
                }
                for feature in rule
                    .get_package_declarations()
                    .get_package_args()
                    .features()
                    .to_string_list()
                {
                    let mut output_elem = Element::new("rule-default-setting");
                    output_elem.set_attribute("name", &feature);
                    elem.append_child(output_elem);
                }
            }
            Target::PackageGroup(package_group) => {
                elem = Element::new("package-group");
                elem.set_attribute("name", package_group.get_name());
                let mut includes = create_value_element(
                    Type::LabelList,
                    &[Some(AttrValue::LabelList(
                        package_group.get_includes().to_vec(),
                    ))],
                    label_printer,
                );
                includes.set_attribute("name", "includes");
                elem.append_child(includes);
                let mut packages = create_value_element(
                    Type::StringList,
                    &[Some(AttrValue::StringList(
                        package_group
                            .get_contained_packages(state.package_group_includes_double_slash),
                    ))],
                    label_printer,
                );
                packages.set_attribute("name", "packages");
                elem.append_child(packages);
            }
            Target::OutputFile(output_file) => {
                elem = Element::new("generated-file");
                elem.set_attribute(
                    "generating-rule",
                    &label_printer.to_string(output_file.get_generating_rule().get_label()),
                );
            }
            Target::InputFile(input_file) => {
                elem = Element::new("source-file");
                if input_file.is_build_file() {
                    self.add_starlark_files_to_element(&mut elem, target)?;
                    add_features_to_element(&mut elem, input_file);
                    elem.set_attribute(
                        "package_contains_errors",
                        if input_file.get_package_declarations().contains_errors() {
                            "true"
                        } else {
                            "false"
                        },
                    );
                }

                // TODO(bazel-team): We're being inconsistent about whether we include the
                // package's default_visibility in the target. For files we do, but for rules we
                // don't.
                add_package_groups_to_element(&mut elem, target, label_printer);
            }
            Target::EnvironmentGroup(env_group) => {
                elem = Element::new("environment-group");
                elem.set_attribute("name", env_group.get_name());
                let mut environments = create_value_element(
                    Type::LabelList,
                    &[Some(AttrValue::LabelList(
                        env_group
                            .get_environment_labels()
                            .get_environments()
                            .iter()
                            .cloned()
                            .collect(),
                    ))],
                    label_printer,
                );
                environments.set_attribute("name", "environments");
                elem.append_child(environments);
                let mut defaults = create_value_element(
                    Type::LabelList,
                    &[Some(AttrValue::LabelList(
                        env_group
                            .get_environment_labels()
                            .get_defaults()
                            .iter()
                            .cloned()
                            .collect(),
                    ))],
                    label_printer,
                );
                defaults.set_attribute("name", "defaults");
                elem.append_child(defaults);
            }
            Target::FakeLoadTarget(_) => {
                elem = Element::new("source-file");
            }
        }

        elem.set_attribute("name", &label_printer.to_string(target.get_label()));
        let mut location = format_utils::get_location(target, state.relative_locations);
        if !state.xml_line_numbers
            && let Some(first_colon) = location.find(':')
        {
            location.truncate(first_colon);
        }

        elem.set_attribute("location", &location);
        Ok(elem)
    }

    // Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:281 XmlOutputFormatter.addStarlarkFilesToElement
    fn add_starlark_files_to_element(
        &self,
        parent: &mut Element,
        build_file: &Target,
    ) -> Result<(), InterruptedException> {
        let dependencies = self
            .state
            .aspect_resolver
            .compute_build_file_dependencies(build_file)?;

        for starlark_file_dep in dependencies {
            let mut elem = Element::new("load");
            elem.set_attribute("name", &self.label_printer.to_string(&starlark_file_dep));
            parent.append_child(elem);
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:257 XmlOutputFormatter.addPackageGroupsToElement
fn add_package_groups_to_element(
    parent: &mut Element,
    target: &Target,
    label_printer: &LabelPrinter,
) {
    for visibility_dependency in target.get_visibility_dependency_labels() {
        let mut elem = Element::new("package-group");
        elem.set_attribute("name", &label_printer.to_string(&visibility_dependency));
        parent.append_child(elem);
    }

    for visibility_declaration in target.get_visibility_declared_labels() {
        let mut elem = Element::new("visibility-label");
        elem.set_attribute("name", &label_printer.to_string(&visibility_declaration));
        parent.append_child(elem);
    }
}

// Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:272 XmlOutputFormatter.addFeaturesToElement
fn add_features_to_element(parent: &mut Element, input_file: &blaze_packages::target::InputFile) {
    for feature in input_file
        .get_package_declarations()
        .get_package_args()
        .features()
        .to_string_list()
    {
        let mut elem = Element::new("feature");
        elem.set_attribute("name", &feature);
        parent.append_child(elem);
    }
}

/// The elements of a list-typed value (Java `(Collection<?>) value`).
fn list_elements(value: Option<&AttrValue>) -> Vec<Option<AttrValue>> {
    match value {
        Some(AttrValue::StringList(v)) => {
            v.iter().cloned().map(AttrValue::String).map(Some).collect()
        }
        Some(AttrValue::StringSet(v)) => {
            v.iter().cloned().map(AttrValue::String).map(Some).collect()
        }
        Some(AttrValue::IntList(v)) => v.iter().copied().map(AttrValue::Int).map(Some).collect(),
        Some(AttrValue::LabelList(v)) => {
            v.iter().cloned().map(AttrValue::Label).map(Some).collect()
        }
        other => panic!("ClassCastException: {other:?} is not a Collection"),
    }
}

/// The entries of a dict-typed value (Java `((Map<?, ?>) value).entrySet()`).
fn dict_entries(value: Option<&AttrValue>) -> Vec<(AttrValue, AttrValue)> {
    match value {
        Some(AttrValue::StringDict(m)) => m
            .iter()
            .map(|(k, v)| (AttrValue::String(k.clone()), AttrValue::String(v.clone())))
            .collect(),
        Some(AttrValue::StringListDict(m)) => m
            .iter()
            .map(|(k, v)| {
                (
                    AttrValue::String(k.clone()),
                    AttrValue::StringList(v.clone()),
                )
            })
            .collect(),
        Some(AttrValue::LabelDictUnary(m)) => m
            .iter()
            .map(|(k, v)| (AttrValue::String(k.clone()), AttrValue::Label(v.clone())))
            .collect(),
        Some(AttrValue::LabelKeyedStringDict(m)) => m
            .iter()
            .map(|(k, v)| (AttrValue::Label(k.clone()), AttrValue::String(v.clone())))
            .collect(),
        Some(AttrValue::LabelListDict(m)) => m
            .iter()
            .map(|(k, v)| {
                (
                    AttrValue::String(k.clone()),
                    AttrValue::LabelList(v.clone()),
                )
            })
            .collect(),
        other => panic!("ClassCastException: {other:?} is not a Map"),
    }
}

/// Java `value.toString()` of a scalar attribute value.
fn scalar_to_string(value: &AttrValue) -> String {
    match value {
        AttrValue::Int(i) => i.to_string(),
        AttrValue::String(s) => s.clone(),
        AttrValue::Bool(b) => b.to_string(),
        AttrValue::TriState(t) => match t {
            blaze_packages::types::TriState::Yes => "YES",
            blaze_packages::types::TriState::No => "NO",
            blaze_packages::types::TriState::Auto => "AUTO",
        }
        .to_owned(),
        other => blaze_util::unported!(
            "XmlOutputFormatter.createValueElement",
            "toString of {other:?}"
        ),
    }
}

/// Creates and returns a new DOM tree for the specified attribute values. For non-configurable
/// attributes, this is a single value. For configurable attributes, this contains one value for
/// each configuration. (Only toplevel values are named attributes; list elements are unnamed.)
///
/// In the case of configurable attributes, multi-value attributes (e.g. lists) merge all
/// configured lists into an aggregate flattened list. Single-value attributes simply refrain to
/// set a value and annotate the DOM element as configurable.
// Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:306 XmlOutputFormatter.createValueElement
fn create_value_element(
    type_: Type,
    values: &[Option<AttrValue>],
    label_printer: &LabelPrinter,
) -> Element {
    let mut elem;
    let has_multiple_values = values.len() > 1;
    if let Some(elem_type) = type_.get_list_element_type() {
        // it's a list (includes "distribs")
        elem = Element::new("list");
        for value in values {
            for elem_value in list_elements(value.as_ref()) {
                elem.append_child(create_value_element(
                    elem_type,
                    &[elem_value],
                    label_printer,
                ));
            }
        }
    } else if type_.is_dict_type() {
        let key_type = type_.get_key_type().unwrap();
        let value_type = type_.get_value_type().unwrap();
        let mut visited_values: Vec<AttrValue> = Vec::new();
        elem = Element::new("dict");
        for value in values {
            for (key, entry_value) in dict_entries(value.as_ref()) {
                if !visited_values.contains(&key) {
                    visited_values.push(key.clone());
                    let mut pair_elem = Element::new("pair");
                    pair_elem.append_child(create_value_element(
                        key_type,
                        &[Some(key)],
                        label_printer,
                    ));
                    pair_elem.append_child(create_value_element(
                        value_type,
                        &[Some(entry_value)],
                        label_printer,
                    ));
                    elem.append_child(pair_elem);
                }
            }
        }
    } else if type_ == Type::License {
        elem = create_single_value_element("license", has_multiple_values);
        if !has_multiple_values {
            let license = match values {
                [Some(AttrValue::License(license))] => license,
                other => panic!("ClassCastException: {other:?} is not a License"),
            };

            let mut exceptions = create_value_element(
                Type::LabelList,
                &[Some(AttrValue::LabelList(
                    license.get_exceptions().to_vec(),
                ))],
                label_printer,
            );
            exceptions.set_attribute("name", "exceptions");
            elem.append_child(exceptions);

            let mut license_types = create_value_element(
                Type::StringList,
                &[Some(AttrValue::StringList(
                    license
                        .get_license_types()
                        .iter()
                        .map(|t| t.name().to_owned())
                        .collect(),
                ))],
                label_printer,
            );
            license_types.set_attribute("name", "license-types");
            elem.append_child(license_types);
        }
    } else {
        // INTEGER STRING LABEL OUTPUT
        elem = create_single_value_element(&type_.to_string(), has_multiple_values);
        if !has_multiple_values && !values.is_empty() {
            // Values such as those of attribute "linkstamp" may be null.
            if let Some(value) = &values[0] {
                // PORT: setAttribute's DOMException ("[[[ERROR: could not be encoded as XML]]]")
                // is raised only for invalid names, never for these values.
                match value {
                    AttrValue::Label(label) => {
                        elem.set_attribute("value", &label_printer.to_string(label))
                    }
                    other => elem.set_attribute("value", &scalar_to_string(other)),
                }
            }
        }
    }
    elem
}

/// Creates the given DOM element, adding `configurable="yes"` if it represents a configurable
/// single-value attribute (configurable list attributes simply have their lists merged into an
/// aggregate flat list).
// Java: com/google/devtools/build/lib/query2/query/output/XmlOutputFormatter.java:378 XmlOutputFormatter.createSingleValueElement
fn create_single_value_element(name: &str, configurable: bool) -> Element {
    let mut elem = Element::new(name);
    if configurable {
        elem.set_attribute("configurable", "yes");
    }
    elem
}

/// A DOM element with attributes and child elements (Java `org.w3c.dom.Element`; the formatter
/// creates no text nodes).
#[derive(Debug)]
struct Element {
    name: String,
    /// Kept sorted by name, like Xerces' `AttributeMap` (`NamedNodeMapImpl.findNamePoint`).
    attributes: Vec<(String, String)>,
    children: Vec<Element>,
}

impl Element {
    fn new(name: &str) -> Self {
        Element {
            name: name.to_owned(),
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    fn set_attribute(&mut self, name: &str, value: &str) {
        match self
            .attributes
            .binary_search_by(|(n, _)| n.as_str().cmp(name))
        {
            Ok(i) => self.attributes[i].1 = value.to_owned(),
            Err(i) => self
                .attributes
                .insert(i, (name.to_owned(), value.to_owned())),
        }
    }

    fn append_child(&mut self, child: Element) {
        self.children.push(child);
    }
}

/// `TransformerFactory.newInstance().newTransformer()` with `INDENT=yes` writing the document
/// (XML version 1.1) to a `StreamResult`: the JDK's Xalan serializer (`ToXMLStream`). The
/// result is a string of chars, each written as UTF-8 (the internal strings' chars, so non-ASCII
/// bytes come out double-encoded, as in Java).
///
/// What this reproduces (checked against the oracle, `query --output=xml`): the declaration
/// `<?xml version="1.1" encoding="UTF-8" standalone="no"?>` and a line break, four spaces of
/// indentation per level, `<e .../>` for elements without children, attributes in name order,
/// and the attribute escapes `&amp; &lt; &gt; &quot;` plus character references for the C0
/// controls (`&#10;` for a line feed, `&#9;`, `&#13;`, `&#1;`); DEL and C1 chars are written
/// as they are.
fn serialize_document(root: &Element) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.1\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    serialize_element(root, 0, &mut out);
    out
}

fn serialize_element(elem: &Element, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("    ");
    }
    out.push('<');
    out.push_str(&elem.name);
    for (name, value) in &elem.attributes {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape_attribute_value(value, out);
        out.push('"');
    }
    if elem.children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for child in &elem.children {
        serialize_element(child, depth + 1, out);
    }
    for _ in 0..depth {
        out.push_str("    ");
    }
    out.push_str("</");
    out.push_str(&elem.name);
    out.push_str(">\n");
}

/// Xalan `ToStream.writeAttrString` for XML 1.1.
fn escape_attribute_value(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{0}' => blaze_util::unported!(
                "ToStream.writeAttrString",
                "a NUL char in an XML attribute value"
            ),
            c if (c as u32) < 0x20 => {
                out.push_str("&#");
                out.push_str(&(c as u32).to_string());
                out.push(';');
            }
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected bytes from the oracle (`query //pkg:g --output=xml` on a genrule whose cmd is
    // "echo 'a<b>&\"c' \n\t\001\177 x é € \r end").
    #[test]
    fn attribute_escapes_match_the_oracle() {
        let mut out = String::new();
        escape_attribute_value(
            "echo 'a<b>&\"c' \n\t\u{1}\u{7f} x \u{c3}\u{a9} \r end",
            &mut out,
        );
        assert_eq!(
            out,
            "echo 'a&lt;b&gt;&amp;&quot;c' &#10;&#9;&#1;\u{7f} x \u{c3}\u{a9} &#13; end"
        );
    }

    #[test]
    fn document_layout() {
        let mut root = Element::new("query");
        root.set_attribute("version", "2");
        let mut rule = Element::new("rule");
        rule.set_attribute("name", "//:a");
        rule.set_attribute("class", "filegroup");
        root.append_child(rule);
        assert_eq!(
            serialize_document(&root),
            "<?xml version=\"1.1\" encoding=\"UTF-8\" standalone=\"no\"?>\n<query version=\"2\">\n    <rule class=\"filegroup\" name=\"//:a\"/>\n</query>\n"
        );
    }
}
