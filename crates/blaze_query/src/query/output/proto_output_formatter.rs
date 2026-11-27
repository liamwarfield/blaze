//! Ports of `query2/query/output/ProtoOutputFormatter.java`,
//! `StreamedProtoOutputFormatter.java` and `StreamedJSONProtoOutputFormatter.java`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::attribute::{
    AggregatingAttributeMapper, Attribute, AttributeInfoProvider, AttributeMap,
};
use blaze_packages::target::{LabelPrinter, Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::JavaMessage;
use blaze_protos::blaze_query::target::Discriminator as TargetDiscriminator;
use blaze_protos::blaze_query::{
    Attribute as AttributePb, EnvironmentGroup as EnvironmentGroupPb, GeneratedFile,
    PackageGroup as PackageGroupPb, Rule as RulePb, SourceFile, Target as TargetPb,
};
use blaze_protos::json::JsonFormat;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, InterruptedException, OutputFormatterCallback, QueryError,
    SynchronizedDelegatingOutputFormatterCallback, ThreadSafeOutputFormatterCallback,
};
use blaze_util::string_encoding::internal_to_unicode;
use blaze_vfs::DigestHashFunction;
use indexmap::{IndexMap, IndexSet};

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::compat::attribute_formatter;
use crate::compat::java_hash_map::java_hash_map_order;
use crate::compat::proto_utils::get_discriminator_from_type;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::output::format_utils;
use crate::query::output::output_formatter::{
    OutputFormatter, OutputStream, StreamedFormatter, write_all,
};
use crate::query::output::possible_attribute_values;
use crate::query::output::synthetic_attribute_hash_calculator::SyntheticAttributeHashCalculator;

// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:90 ProtoOutputFormatter.RULE_IMPLEMENTATION_HASH_ATTR_NAME
pub const RULE_IMPLEMENTATION_HASH_ATTR_NAME: &str = "$rule_implementation_hash";

/// Java `SCALAR_TYPES`.
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:95 ProtoOutputFormatter.SCALAR_TYPES
fn is_scalar_type(type_: Type) -> bool {
    matches!(
        type_,
        Type::Integer
            | Type::String
            | Type::Label
            | Type::NodepLabel
            | Type::DormantLabel
            | Type::Output
            | Type::Boolean
            | Type::Tristate
            | Type::License
    )
}

/// Java's `Predicate<String> ruleAttributePredicate`.
#[derive(Clone)]
enum AttributePredicate {
    AlwaysTrue,
    AlwaysFalse,
    In(HashSet<String>),
}

impl AttributePredicate {
    fn apply(&self, name: &str) -> bool {
        match self {
            AttributePredicate::AlwaysTrue => true,
            AttributePredicate::AlwaysFalse => false,
            AttributePredicate::In(set) => set.contains(name),
        }
    }
}

// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:160 ProtoOutputFormatter.newAttributePredicate
fn new_attribute_predicate(output_attributes: &[String]) -> AttributePredicate {
    if output_attributes == ["all"] {
        AttributePredicate::AlwaysTrue
    } else if output_attributes.is_empty() {
        AttributePredicate::AlwaysFalse
    } else {
        AttributePredicate::In(output_attributes.iter().cloned().collect())
    }
}

/// The fields `setOptions` and `setEventHandler` set (with Java's initial values).
#[derive(Clone)]
pub struct ProtoFormatterState {
    aspect_resolver: Option<Arc<dyn AspectResolver>>,
    dependency_filter: Option<DependencyFilter>,
    package_group_includes_double_slash: bool,
    relative_locations: bool,
    include_default_values: bool,
    rule_attribute_predicate: AttributePredicate,
    flatten_selects: bool,
    include_locations: bool,
    include_rule_inputs_and_outputs: bool,
    include_synthetic_attribute_hash: bool,
    include_instantiation_stack: bool,
    include_definition_stack: bool,
    include_starlark_rule_env: bool,
    include_attribute_source_aspects: bool,
    hash_function: Option<DigestHashFunction>,
    /// Non-null if and only if --proto:rule_classes option is set: Java's
    /// `RuleClassInfoFormatter`, the rule class keys already given a `rule_class_info`
    /// (shared by the converters made from this state).
    rule_class_info_formatter: Option<Arc<std::sync::Mutex<std::collections::HashSet<String>>>>,
    event_handler: Option<Arc<dyn EventHandler>>,
}

impl Default for ProtoFormatterState {
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:108 ProtoOutputFormatter field initializers
    fn default() -> Self {
        ProtoFormatterState {
            aspect_resolver: None,
            dependency_filter: None,
            package_group_includes_double_slash: false,
            relative_locations: false,
            include_default_values: true,
            rule_attribute_predicate: AttributePredicate::AlwaysTrue,
            flatten_selects: true,
            include_locations: true,
            include_rule_inputs_and_outputs: true,
            include_synthetic_attribute_hash: false,
            include_instantiation_stack: false,
            include_definition_stack: false,
            include_starlark_rule_env: true,
            include_attribute_source_aspects: false,
            hash_function: None,
            rule_class_info_formatter: None,
            event_handler: None,
        }
    }
}

/// Which of the three proto formatters this is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProtoKind {
    /// `ProtoOutputFormatter` (`proto`).
    Proto,
    /// `StreamedProtoOutputFormatter` (`streamed_proto`).
    StreamedProto,
    /// `StreamedJSONProtoOutputFormatter` (`streamed_jsonproto`).
    StreamedJsonProto,
}

/// An output formatter that outputs a protocol buffer representation of a query result and
/// outputs the proto bytes to the output print stream. By taking the bytes and calling
/// `mergeFrom()` on a `Build.QueryResult` object the full result can be reconstructed.
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:84 ProtoOutputFormatter
pub struct ProtoOutputFormatter {
    kind: ProtoKind,
    state: RefCell<ProtoFormatterState>,
}

impl ProtoOutputFormatter {
    pub fn new(kind: ProtoKind) -> Self {
        ProtoOutputFormatter {
            kind,
            state: RefCell::new(ProtoFormatterState::default()),
        }
    }

    /// The `toTargetProtoBuffer` logic with the current options.
    pub fn target_proto_converter(&self) -> TargetProtoConverter {
        TargetProtoConverter {
            state: self.state.borrow().clone(),
        }
    }
}

impl OutputFormatter for ProtoOutputFormatter {
    fn get_name(&self) -> &'static str {
        match self.kind {
            // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:129 ProtoOutputFormatter.getName
            ProtoKind::Proto => "proto",
            // Java: com/google/devtools/build/lib/query2/query/output/StreamedProtoOutputFormatter.java:29 StreamedProtoOutputFormatter.getName
            ProtoKind::StreamedProto => "streamed_proto",
            // Java: com/google/devtools/build/lib/query2/query/output/StreamedJSONProtoOutputFormatter.java:30 StreamedJSONProtoOutputFormatter.getName
            ProtoKind::StreamedJsonProto => "streamed_jsonproto",
        }
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for ProtoOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:134 ProtoOutputFormatter.setOptions
    fn set_options(
        &self,
        options: &QueryOptions,
        aspect_resolver: Arc<dyn AspectResolver>,
        hash_function: DigestHashFunction,
    ) {
        let mut state = self.state.borrow_mut();
        state.aspect_resolver = Some(aspect_resolver);
        state.dependency_filter = Some(format_utils::get_dependency_filter(options));
        state.package_group_includes_double_slash =
            options.incompatible_package_group_includes_double_slash;
        state.relative_locations = options.relative_locations;
        state.include_default_values = options.proto_include_default_values;
        state.rule_attribute_predicate =
            new_attribute_predicate(&options.proto_output_rule_attributes);
        state.flatten_selects = options.proto_flatten_selects;
        state.include_locations = options.proto_include_locations;
        state.include_rule_inputs_and_outputs = options.proto_include_rule_inputs_and_outputs;
        state.include_synthetic_attribute_hash = options.proto_include_synthetic_attribute_hash;
        state.include_instantiation_stack = options.proto_include_instantiation_stack;
        state.include_definition_stack = options.proto_include_definition_stack;
        state.include_attribute_source_aspects = options.proto_include_attribute_source_aspects;
        state.include_starlark_rule_env = options.proto_include_starlark_rule_env;
        state.hash_function = Some(hash_function);
        // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:152 ProtoOutputFormatter.setOptions
        state.rule_class_info_formatter = options
            .proto_rule_classes
            .then(|| Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())));
    }

    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:155 ProtoOutputFormatter.setEventHandler
    fn set_event_handler(&self, event_handler: Option<Arc<dyn EventHandler>>) {
        self.state.borrow_mut().event_handler = event_handler;
    }

    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:177 ProtoOutputFormatter.createStreamCallback
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

    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        _options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        Box::new(ProtoCallback {
            kind: self.kind,
            converter: self.target_proto_converter(),
            out,
            label_printer: label_printer.clone(),
            buffer: RefCell::new(Vec::new()),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

/// The callbacks of the three formatters.
///
/// - `proto`: Java's `StreamedQueryResultFormatter`, which writes each target as field 1
///   (`QueryResult.target`) through a 16 KiB `CodedOutputStream` flushed by `close`.
/// - `streamed_proto`: `writeDelimitedTo` per target.
/// - `streamed_jsonproto`: one compact `JsonFormat` line per target.
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:611 ProtoOutputFormatter.StreamedQueryResultFormatter
struct ProtoCallback {
    kind: ProtoKind,
    converter: TargetProtoConverter,
    out: OutputStream,
    label_printer: LabelPrinter,
    /// The `CodedOutputStream` buffer of `proto`.
    buffer: RefCell<Vec<u8>>,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for ProtoCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for ProtoCallback {
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            let target_pb = self
                .converter
                .to_target_proto_buffer(target, &self.label_printer)?;
            match self.kind {
                // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:628 StreamedQueryResultFormatter.processOutput
                ProtoKind::Proto => {
                    // QueryResult.TARGET_FIELD_NUMBER
                    blaze_protos::java::write_message_field(
                        1,
                        &target_pb,
                        &mut self.buffer.borrow_mut(),
                    );
                }
                // Java: com/google/devtools/build/lib/query2/query/output/StreamedProtoOutputFormatter.java:38 StreamedProtoOutputFormatter.processOutput
                ProtoKind::StreamedProto => {
                    let mut bytes = Vec::new();
                    target_pb.write_delimited_to(&mut bytes);
                    write_all(&self.out, &bytes)?;
                }
                // Java: com/google/devtools/build/lib/query2/query/output/StreamedJSONProtoOutputFormatter.java:41 StreamedJSONProtoOutputFormatter.processOutput
                ProtoKind::StreamedJsonProto => {
                    let json = JsonFormat::printer()
                        .omitting_insignificant_whitespace()
                        .print(&target_pb)
                        .map_err(|e| {
                            OutputFormatterError::from(std::io::Error::other(
                                e.get_message().to_owned(),
                            ))
                        })?;
                    write_all(&self.out, json.as_bytes())?;
                    write_all(&self.out, b"\n")?;
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:641 StreamedQueryResultFormatter.close
    fn close(&self, _fail_fast: bool) -> Result<(), OutputFormatterError> {
        if self.kind == ProtoKind::Proto {
            let bytes = std::mem::take(&mut *self.buffer.borrow_mut());
            write_all(&self.out, &bytes)?;
            self.out.borrow_mut().flush()?;
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

/// `ProtoOutputFormatter.toTargetProtoBuffer` with a snapshot of the formatter's options.
#[derive(Clone)]
pub struct TargetProtoConverter {
    state: ProtoFormatterState,
}

impl TargetProtoConverter {
    fn dependency_filter(&self) -> &DependencyFilter {
        self.state
            .dependency_filter
            .as_ref()
            .expect("NullPointerException: setOptions was not called")
    }

    /// Converts a logical `Target` object into a `Build.Target` protobuffer.
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:184 ProtoOutputFormatter.toTargetProtoBuffer
    pub fn to_target_proto_buffer(
        &self,
        target: &Target,
        label_printer: &LabelPrinter,
    ) -> Result<TargetPb, InterruptedException> {
        self.to_target_proto_buffer_with_extra(target, label_printer, "")
    }

    /// Converts a logical `Target` object into a `Build.Target` protobuffer.
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:190 ProtoOutputFormatter.toTargetProtoBuffer
    pub fn to_target_proto_buffer_with_extra(
        &self,
        target: &Target,
        label_printer: &LabelPrinter,
        extra_data_for_attr_hash: &str,
    ) -> Result<TargetPb, InterruptedException> {
        let state = &self.state;
        let mut target_pb = TargetPb::default();
        match target {
            Target::Rule(rule) => {
                let mut rule_pb = RulePb {
                    name: u(label_printer.to_string(rule.get_label())),
                    rule_class: internal_to_unicode(rule.get_rule_class()).into_owned(),
                    ..Default::default()
                };
                if state.include_locations {
                    rule_pb.location = Some(u(format_utils::get_location(
                        target,
                        state.relative_locations,
                    )));
                }
                self.add_attributes(&mut rule_pb, rule, extra_data_for_attr_hash, label_printer);
                let transitive_digest = rule
                    .get_rule_class_object()
                    .get_rule_definition_environment_digest();
                if let Some(transitive_digest) = transitive_digest
                    && self.include_rule_definition_environment()
                {
                    // The transitive digest can be used to detect changes in the rule's
                    // definition environment.
                    let mut attr = AttributePb {
                        name: RULE_IMPLEMENTATION_HASH_ATTR_NAME.to_owned(),
                        ..Default::default()
                    };
                    attr.set_type(get_discriminator_from_type(Type::String));
                    attr.string_value = Some(
                        transitive_digest
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>(),
                    ); // hexify
                    rule_pb.attribute.push(attr);
                }

                let aspects_dependencies = state
                    .aspect_resolver
                    .as_ref()
                    .expect("NullPointerException: aspectResolver")
                    .compute_aspect_dependencies(target, self.dependency_filter())?;
                if !aspects_dependencies.is_empty() {
                    let mut attributes: Vec<AttributePb> = Vec::new();
                    for (aspect, attribute_labels) in &aspects_dependencies {
                        for (attribute, labels) in attribute_labels.values() {
                            if !self.include_aspect_attribute(attribute, labels) {
                                continue;
                            }
                            let attribute_value =
                                self.get_aspect_attribute_value(target, attribute, labels);
                            let serialized_attribute = attribute_formatter::get_attribute_proto(
                                attribute.get_name(),
                                attribute.get_type(),
                                Some(&attribute_value),
                                /* explicitly_specified= */ false,
                                /* encode_boolean_and_tri_state_as_integer_and_string= */
                                true,
                                /* source_aspect= */ Some(aspect.get_name()),
                                state.include_attribute_source_aspects,
                                label_printer,
                            );
                            attributes.push(serialized_attribute);
                        }
                    }
                    rule_pb
                        .attribute
                        .extend(distinct_sorted_by_name(attributes));
                }
                if state.include_rule_inputs_and_outputs {
                    if !aspects_dependencies.is_empty() {
                        let mut seen: IndexSet<Label> = IndexSet::new();
                        for (_, attribute_labels) in &aspects_dependencies {
                            for (_, labels) in attribute_labels.values() {
                                for dep in labels {
                                    seen.insert(dep.clone());
                                }
                            }
                        }
                        for dep in seen {
                            rule_pb.rule_input.push(u(label_printer.to_string(&dep)));
                        }
                    }
                    for input in rule.get_sorted_labels(self.dependency_filter()) {
                        rule_pb.rule_input.push(u(label_printer.to_string(&input)));
                    }
                    let mut seen_outputs: Vec<Label> = Vec::new();
                    for output in rule.get_output_files() {
                        if !seen_outputs.contains(output.get_label()) {
                            seen_outputs.push(output.get_label().clone());
                            rule_pb
                                .rule_output
                                .push(u(label_printer.to_string(output.get_label())));
                        }
                    }
                }
                for feature in rule
                    .get_package_declarations()
                    .get_package_args()
                    .features()
                    .to_string_list()
                {
                    rule_pb.default_setting.push(u(feature));
                }

                if state.include_instantiation_stack {
                    for fr in rule.reconstruct_call_stack() {
                        // Always report relative locations.
                        // (New fields needn't honor relativeLocations.)
                        rule_pb
                            .instantiation_stack
                            .push(u(call_stack_entry_string(&fr, rule.get_package_metadata())));
                    }
                }

                if state.include_definition_stack && rule.get_rule_class_object().is_starlark() {
                    for fr in rule.get_rule_class_object().get_call_stack() {
                        // Always report relative locations.
                        // (New fields needn't honor relativeLocations.)
                        rule_pb
                            .definition_stack
                            .push(u(call_stack_entry_string(fr, rule.get_package_metadata())));
                    }
                }

                if let Some(rule_class_keys) = &state.rule_class_info_formatter {
                    add_rule_class_key_and_info_if_needed(rule_class_keys, &mut rule_pb, rule);
                }

                target_pb.set_type(TargetDiscriminator::Rule);
                target_pb.rule = Some(rule_pb);
            }
            Target::OutputFile(output_file) => {
                let label = output_file.get_label();
                let generating_rule = output_file.get_generating_rule();
                let mut output = GeneratedFile {
                    generating_rule: u(label_printer.to_string(generating_rule.get_label())),
                    name: u(label_printer.to_string(label)),
                    ..Default::default()
                };

                if state.include_locations {
                    output.location = Some(u(format_utils::get_location(
                        target,
                        state.relative_locations,
                    )));
                }
                target_pb.set_type(TargetDiscriminator::GeneratedFile);
                target_pb.generated_file = Some(output);
            }
            Target::InputFile(input_file) => {
                let label = input_file.get_label();

                let mut input = SourceFile {
                    name: u(label_printer.to_string(label)),
                    ..Default::default()
                };
                if state.include_locations {
                    input.location = Some(u(format_utils::get_location(
                        target,
                        state.relative_locations,
                    )));
                }

                if input_file.is_build_file() {
                    let starlark_load_labels = match &state.aspect_resolver {
                        None => input_file
                            .get_package_declarations()
                            .get_or_compute_transitively_loaded_starlark_files(),
                        Some(aspect_resolver) => {
                            aspect_resolver.compute_build_file_dependencies(target)?
                        }
                    };

                    for starlark_load_label in &starlark_load_labels {
                        input
                            .subinclude
                            .push(u(label_printer.to_string(starlark_load_label)));
                    }

                    for feature in input_file
                        .get_package_declarations()
                        .get_package_args()
                        .features()
                        .to_string_list()
                    {
                        input.feature.push(u(feature));
                    }

                    input.package_contains_errors =
                        Some(input_file.get_package_declarations().contains_errors());
                }

                for visibility_dependency in target.get_visibility_dependency_labels() {
                    input
                        .package_group
                        .push(u(label_printer.to_string(&visibility_dependency)));
                }

                for visibility_declaration in target.get_visibility_declared_labels() {
                    input
                        .visibility_label
                        .push(u(label_printer.to_string(&visibility_declaration)));
                }

                target_pb.set_type(TargetDiscriminator::SourceFile);
                target_pb.source_file = Some(input);
            }
            Target::FakeLoadTarget(_) => {
                let label = target.get_label();
                let mut input = SourceFile {
                    name: u(label_printer.to_string(label)),
                    ..Default::default()
                };

                if state.include_locations {
                    input.location = Some(u(format_utils::get_location(
                        target,
                        state.relative_locations,
                    )));
                }
                target_pb.set_type(TargetDiscriminator::SourceFile);
                target_pb.source_file = Some(input);
            }
            Target::PackageGroup(package_group) => {
                let mut package_group_pb = PackageGroupPb {
                    name: u(label_printer.to_string(package_group.get_label())),
                    ..Default::default()
                };
                for contained_package in
                    package_group.get_contained_packages(state.package_group_includes_double_slash)
                {
                    package_group_pb
                        .contained_package
                        .push(u(contained_package));
                }
                for include in package_group.get_includes() {
                    package_group_pb
                        .included_package_group
                        .push(u(label_printer.to_string(include)));
                }

                target_pb.set_type(TargetDiscriminator::PackageGroup);
                target_pb.package_group = Some(package_group_pb);
            }
            Target::EnvironmentGroup(env_group) => {
                let mut env_group_pb = EnvironmentGroupPb {
                    name: u(label_printer.to_string(env_group.get_label())),
                    ..Default::default()
                };
                for env in env_group.get_environment_labels().get_environments() {
                    env_group_pb
                        .environment
                        .push(u(label_printer.to_string(env)));
                }
                for default_env in env_group.get_environment_labels().get_defaults() {
                    env_group_pb
                        .default
                        .push(u(label_printer.to_string(default_env)));
                }
                target_pb.set_type(TargetDiscriminator::EnvironmentGroup);
                target_pb.environment_group = Some(env_group_pb);
            }
        }

        Ok(target_pb)
    }

    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:401 ProtoOutputFormatter.addAttributes
    fn add_attributes(
        &self,
        rule_pb: &mut RulePb,
        rule: &Rule,
        extra_data_for_attr_hash: &str,
        label_printer: &LabelPrinter,
    ) {
        let state = &self.state;
        let mut serialized_attributes: Vec<AttributePb> = Vec::new();
        // Java `Map<Attribute, Build.Attribute> serializedAttributes` (by attribute name).
        let mut serialized_attributes_by_name: HashMap<String, AttributePb> = HashMap::new();
        let attribute_mapper = AggregatingAttributeMapper::of(rule);
        for attr in rule.get_attributes() {
            if !self.should_include_attribute(rule, attr) {
                continue;
            }
            let attribute_value: Option<AttrValue> = if state.flatten_selects
                || !attribute_mapper.base().is_configurable(attr.get_name())
            {
                get_flattened_attribute_values(attr.get_type(), rule, attr)
            } else {
                attribute_mapper
                    .get_selector_list(attr.get_name(), attr.get_type())
                    .map(AttrValue::SelectorList)
            };
            let serialized_attribute = attribute_formatter::get_attribute_proto(
                attr.get_name(),
                attr.get_type(),
                attribute_value.as_ref(),
                rule.is_attribute_value_explicitly_specified_for(attr),
                /* encode_boolean_and_tri_state_as_integer_and_string= */ true,
                /* source_aspect= */ None,
                state.include_attribute_source_aspects,
                label_printer,
            );
            serialized_attributes_by_name
                .insert(attr.get_name().to_owned(), serialized_attribute.clone());
            serialized_attributes.push(serialized_attribute);
        }

        rule_pb
            .attribute
            .extend(distinct_sorted_by_name(serialized_attributes));
        if state.include_synthetic_attribute_hash {
            let mut attr = AttributePb {
                name: "$internal_attr_hash".to_owned(),
                string_value: Some(SyntheticAttributeHashCalculator::compute(
                    rule,
                    &serialized_attributes_by_name,
                    extra_data_for_attr_hash,
                    state
                        .hash_function
                        .expect("NullPointerException: hashFunction"),
                    state.include_attribute_source_aspects,
                    state.include_starlark_rule_env,
                )),
                ..Default::default()
            };
            attr.set_type(get_discriminator_from_type(Type::String));
            rule_pb.attribute.push(attr);
        }
    }

    /// Do not include attributes which take their default value, if requested by the options.
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:451 ProtoOutputFormatter.shouldIncludeAttribute
    fn should_include_attribute(&self, rule: &Rule, attr: &Attribute) -> bool {
        (self.state.include_default_values
            || rule.is_attribute_value_explicitly_specified_for(attr))
            && self.state.rule_attribute_predicate.apply(attr.get_name())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:456 ProtoOutputFormatter.getAspectAttributeValue
    fn get_aspect_attribute_value(
        &self,
        target: &Target,
        attribute: &Attribute,
        labels: &IndexSet<Label>,
    ) -> AttrValue {
        let attribute_type = attribute.get_type();
        if attribute_type == Type::Label {
            blaze_util::check_state!(
                labels.len() == 1,
                "attribute={}, labels={:?}",
                attribute.get_name(),
                labels
            );
            AttrValue::Label(labels[0].clone())
        } else if attribute_type == Type::LabelKeyedStringDict {
            if let Some(event_handler) = &self.state.event_handler {
                event_handler.handle(&Event::error(format!(
                    "Target \"{}\", aspect attribute \"{}\": type \"{}\" not yet supported with \
                     --output=proto.",
                    target.get_label(),
                    attribute.get_name(),
                    "dict(label, string)"
                )));
            }
            AttrValue::LabelKeyedStringDict(IndexMap::new())
        } else {
            blaze_util::check_state!(
                attribute_type == Type::LabelList,
                "attribute={}, type={:?}, labels={:?}",
                attribute.get_name(),
                attribute_type,
                labels
            );
            AttrValue::LabelList(labels.iter().cloned().collect())
        }
    }

    /// Specifies whether the given aspect attribute and its value should be included in the
    /// output.
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:497 ProtoOutputFormatter.includeAspectAttribute
    fn include_aspect_attribute(&self, _attr: &Attribute, _value: &IndexSet<Label>) -> bool {
        true
    }

    /// Specifies whether the rule definition environment hash should be included in the output.
    // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:501 ProtoOutputFormatter.includeRuleDefinitionEnvironment
    fn include_rule_definition_environment(&self) -> bool {
        true
    }
}

/// Java `StringEncoding.internalToUnicode(s)` on an owned internal string (the proto fields of
/// `toTargetProtoBuffer` hold Unicode text).
/// Adds the rule class key to the rule, and the rule class info the first time the key is
/// seen (Java `RuleClassInfoFormatter`, whose `ExtractorContext` renders labels with
/// `LabelRenderer.DEFAULT` and extracts natively defined attributes).
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:590 ProtoOutputFormatter.RuleClassInfoFormatter.addRuleClassKeyAndInfoIfNeeded
fn add_rule_class_key_and_info_if_needed(
    rule_class_keys: &std::sync::Mutex<std::collections::HashSet<String>>,
    rule_pb: &mut RulePb,
    rule: &blaze_packages::target::Rule,
) {
    let rule_class_key = rule.get_rule_class_object().get_key();
    rule_pb.rule_class_key = Some(internal_to_unicode(rule_class_key).into_owned());
    if rule_class_keys
        .lock()
        .unwrap()
        .insert(rule_class_key.to_owned())
    {
        // Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:580 ProtoOutputFormatter.RuleClassInfoFormatter.extractorContext
        let mut extractor_context =
            blaze_starlarkdocextract::extractor_context::ExtractorContext::new(
                blaze_starlarkdocextract::label_renderer::LabelRenderer::default_renderer(),
            );
        extractor_context.extract_natively_defined_attrs = true;
        // TODO(b/368091415): instead of rule.getRuleClass(), we should be using the rule's
        // public name.
        rule_pb.rule_class_info = Some(
            blaze_starlarkdocextract::rule_info_extractor::build_rule_info(
                &extractor_context,
                rule.get_rule_class(),
                rule.get_rule_class_object(),
            ),
        );
    }
}

fn u(s: String) -> String {
    match internal_to_unicode(&s) {
        Cow::Borrowed(_) => s,
        Cow::Owned(unicode) => unicode,
    }
}

/// `attributes.stream().distinct().sorted(ATTRIBUTE_NAME)`: drops equal protos (keeping the
/// first) and stably sorts by name (Java `String.compareTo`; attribute names are ASCII).
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:92 ProtoOutputFormatter.ATTRIBUTE_NAME
fn distinct_sorted_by_name(attributes: Vec<AttributePb>) -> Vec<AttributePb> {
    let mut distinct: Vec<AttributePb> = Vec::with_capacity(attributes.len());
    for a in attributes {
        if !distinct.contains(&a) {
            distinct.push(a);
        }
    }
    distinct.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    distinct
}

/// Returns the attribute value as a single value (`None` is Java's null): the only possible
/// value, or for a configurable attribute with several possible values, the concatenation of
/// list values or the merge of dict values; scalars with several values are null.
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:519 ProtoOutputFormatter.getFlattenedAttributeValues
fn get_flattened_attribute_values(
    attr_type: Type,
    rule: &Rule,
    attr: &Attribute,
) -> Option<AttrValue> {
    let treat_multiple_as_none = is_scalar_type(attr_type);
    let mut possible_values =
        possible_attribute_values::for_rule_and_attribute(rule, attr, treat_multiple_as_none);

    // If there is only a single possible value, return that value.
    if possible_values.len() == 1 {
        return possible_values.pop().unwrap();
    }
    // Otherwise, there are multiple possible values. To conform to the message shape expected by
    // query output's clients, we must transform the list of possible values. This transformation
    // will be lossy, but this is the best we can do.

    // If the attribute's type is not a collection type, return null. Query output's clients do
    // not support list values for scalar attributes.
    if is_scalar_type(attr_type) {
        return None;
    }

    // If the attribute's type is a collection type, merge the list of collections into a single
    // collection. This is a sensible solution for query output's clients, which are happy to
    // get the union of possible values.
    let value = |v: Option<AttrValue>| -> AttrValue {
        v.unwrap_or_else(|| {
            panic!(
                "NullPointerException: possible value of {}",
                attr.get_name()
            )
        })
    };
    match attr_type {
        Type::StringList => {
            let mut builder = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::StringList(list) => builder.extend(list),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::StringList(builder))
        }
        Type::LabelList | Type::NodepLabelList | Type::DormantLabelList | Type::OutputList => {
            let mut builder = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::LabelList(list) => builder.extend(list),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::LabelList(builder))
        }
        Type::IntegerList => {
            let mut builder = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::IntList(list) => builder.extend(list),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::IntList(builder))
        }
        // The merged dict is a java.util.HashMap: PORT: java-hash-order (see java_hash_map).
        Type::StringDict => {
            let mut entries = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::StringDict(dict) => entries.extend(dict),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::StringDict(
                java_hash_map_order(entries).into_iter().collect(),
            ))
        }
        Type::StringListDict => {
            let mut entries = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::StringListDict(dict) => entries.extend(dict),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::StringListDict(
                java_hash_map_order(entries).into_iter().collect(),
            ))
        }
        Type::LabelListDict => {
            let mut entries = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::LabelListDict(dict) => entries.extend(dict),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::LabelListDict(
                java_hash_map_order(entries).into_iter().collect(),
            ))
        }
        Type::LabelDictUnary => {
            let mut entries = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::LabelDictUnary(dict) => entries.extend(dict),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::LabelDictUnary(
                java_hash_map_order(entries).into_iter().collect(),
            ))
        }
        Type::LabelKeyedStringDict => {
            let mut entries = Vec::new();
            for possible_value in possible_values {
                match value(possible_value) {
                    AttrValue::LabelKeyedStringDict(dict) => entries.extend(dict),
                    other => panic!("ClassCastException: {other:?}"),
                }
            }
            Some(AttrValue::LabelKeyedStringDict(
                java_hash_map_order(entries).into_iter().collect(),
            ))
        }
        _ => panic!("AssertionError: Unknown type: {attr_type:?}"),
    }
}

/// `FormatUtils.getRootRelativeLocation(fr.location, rule.getPackageMetadata()) + ": " + fr.name`
/// (a builtin frame's location is `Location.BUILTIN`).
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:273 ProtoOutputFormatter.toTargetProtoBuffer
fn call_stack_entry_string(
    fr: &blaze_starlark::error::StackEntry,
    package_metadata: &blaze_packages::package::PackageMetadata,
) -> String {
    let location = fr
        .location
        .clone()
        .unwrap_or_else(blaze_starlark::Location::builtin);
    format!(
        "{}: {}",
        format_utils::get_root_relative_location(location, Some(package_metadata)),
        fr.name
    )
}
