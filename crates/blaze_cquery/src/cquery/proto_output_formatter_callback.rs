//! Port of `query2/cquery/ProtoOutputFormatterCallback.java` (with its
//! `ConfiguredProtoOutputFormatter` and `ConfigurationCache`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use blaze_analysis::rule::config_util;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::config_output::ConfigurationForOutput;
use blaze_build_config::fragment::FragmentRegistry;
use blaze_optdefs::command::cquery_options::Transitions;
use blaze_packages::attribute::{Attribute, AttributeInfoProvider, AttributeMap};
use blaze_packages::target::{LabelPrinter, Rule, RuleOrMacroInstance, Target};
use blaze_protos::JavaMessage;
use blaze_protos::analysis;
use blaze_protos::blaze_query::{ConfiguredRuleInput, QueryResult, Rule as RulePb};
use blaze_protos::json::JsonFormat;
use blaze_query::compat::attribute_formatter;
use blaze_query::query::aspectresolvers::aspect_resolver::AspectResolver;
use blaze_query::query::output::output_formatter::StreamedFormatter;
use blaze_query::query::output::proto_output_formatter::{ProtoKind, ProtoOutputFormatter};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};
use blaze_vfs::DigestHashFunction;

use crate::cquery::configured_target_query_environment::{
    CqueryNode, configured_attribute_mapper, get_config_conditions,
};
use crate::cquery::cquery_options::as_query_options;
use crate::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// Defines the types of proto output this class can handle.
// Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:66 ProtoOutputFormatterCallback.OutputType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputType {
    BINARY,
    DELIMITED_BINARY,
    TEXT,
    JSON,
}

impl OutputType {
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:78 OutputType.formatName
    pub fn format_name(self) -> &'static str {
        match self {
            OutputType::BINARY => "proto",
            OutputType::DELIMITED_BINARY => "streamed_proto",
            OutputType::TEXT => "textproto",
            OutputType::JSON => "jsonproto",
        }
    }
}

/// Assigns ids to configurations in first-use order.
// Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:83 ProtoOutputFormatterCallback.ConfigurationCache
struct ConfigurationCache {
    /// `LinkedHashMap<BuildConfigurationValue, Integer>`; configurations are equal when their
    /// checksums are.
    cache: Vec<Arc<BuildConfigurationValue>>,
}

impl ConfigurationCache {
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:92 ConfigurationCache.getId
    fn get_id(&mut self, build_configuration_value: &Arc<BuildConfigurationValue>) -> u32 {
        if let Some(i) = self
            .cache
            .iter()
            .position(|c| c.checksum() == build_configuration_value.checksum())
        {
            return (i + 1) as u32;
        }
        self.cache.push(build_configuration_value.clone());
        self.cache.len() as u32
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:102 ConfigurationCache.getConfigurations
    fn get_configurations(&self) -> Vec<analysis::Configuration> {
        self.cache
            .iter()
            .enumerate()
            .map(|(i, v)| create_configuration_proto(v, (i + 1) as u32))
            .collect()
    }
}

// Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:108 ConfigurationCache.createConfigurationProto
fn create_configuration_proto(
    configuration_value: &BuildConfigurationValue,
    id: u32,
) -> analysis::Configuration {
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:136 ConfigurationForOutput.getConfigurationForOutput
    // (the single-argument overload: the fragment definitions are the configuration's own
    // fragments).
    let fragment_defs = FragmentRegistry::create(
        configuration_value
            .get_fragments()
            .iter()
            .map(|(fragment_class, _)| *fragment_class)
            .collect(),
        Vec::new(),
        Vec::new(),
    );
    let configuration_for_output =
        ConfigurationForOutput::get_configuration_for_output(configuration_value, &fragment_defs);
    let mut config_builder = analysis::Configuration::default();

    for fragment_for_output in &configuration_for_output.fragments {
        config_builder.fragments.push(analysis::Fragment {
            name: fragment_for_output.name.clone(),
            fragment_option_names: fragment_for_output.fragment_options.clone(),
        });
    }

    for fragment_options_for_output in &configuration_for_output.fragment_options {
        let mut fragment_options = analysis::FragmentOptions {
            name: fragment_options_for_output.name.clone(),
            options: Vec::new(),
        };
        for (name, value) in &fragment_options_for_output.options {
            fragment_options.options.push(analysis::Option {
                name: Some(name.clone()),
                value: Some(value.clone()),
            });
        }
        config_builder.fragment_options.push(fragment_options);
    }

    let build_event = config_util::to_build_event(configuration_value);
    let checksum = build_event.event_id.clone();
    let config_proto = &build_event.configuration;

    config_builder.checksum = checksum;
    config_builder.mnemonic = config_proto.mnemonic.clone();
    config_builder.platform_name = config_proto.platform_name.clone();
    config_builder.id = id;
    config_builder.is_tool = config_proto.is_tool;
    config_builder
}

/// Proto output formatter for cquery results.
// Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:63 ProtoOutputFormatterCallback
pub struct ProtoOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    cquery_result_builder: RefCell<analysis::CqueryResult>,
    output_type: OutputType,
    resolver: Arc<dyn AspectResolver>,
    hash_function: DigestHashFunction,
    configuration_cache: RefCell<ConfigurationCache>,
    label_printer: LabelPrinter,
}

impl<'a> ProtoOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:166 ProtoOutputFormatterCallback.<init>
    pub fn new(
        base: CqueryThreadsafeCallback<'a>,
        resolver: Arc<dyn AspectResolver>,
        output_type: OutputType,
        label_printer: LabelPrinter,
        hash_function: DigestHashFunction,
    ) -> Self {
        ProtoOutputFormatterCallback {
            base,
            cquery_result_builder: RefCell::new(analysis::CqueryResult::default()),
            output_type,
            resolver,
            hash_function,
            configuration_cache: RefCell::new(ConfigurationCache { cache: Vec::new() }),
            label_printer,
        }
    }

    fn get_configuration_for_options(
        &self,
        options: &Arc<BuildOptions>,
    ) -> Option<Arc<BuildConfigurationValue>> {
        self.base
            .get_configuration(Some(&BuildConfigurationKey::create(options.clone())))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:194 ProtoOutputFormatterCallback.queryResultFromCqueryResult
    fn query_result_from_cquery_result(cquery_result: &analysis::CqueryResult) -> QueryResult {
        let mut query_result = QueryResult::default();
        for ct in &cquery_result.results {
            query_result
                .target
                .push(ct.target.clone().unwrap_or_default());
        }
        query_result
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:244 ProtoOutputFormatterCallback.writeData
    fn write_data<M: JavaMessage>(&self, message: &M) -> std::io::Result<()> {
        let Some(out) = &self.base.out else {
            return Ok(());
        };
        let mut out = out.borrow_mut();
        match self.output_type {
            OutputType::BINARY => {
                out.write_all(&message.to_byte_array())?;
            }
            OutputType::DELIMITED_BINARY => {
                let mut bytes = Vec::new();
                message.write_delimited_to(&mut bytes);
                out.write_all(&bytes)?;
            }
            OutputType::TEXT => {
                blaze_util::unported!("TextFormat.printer().print", "--output=textproto")
            }
            OutputType::JSON => {
                let json = JsonFormat::printer()
                    .print(message)
                    .map_err(|e| std::io::Error::other(e.get_message().to_owned()))?;
                out.write_all(json.as_bytes())?;
                out.write_all(b"\n")?;
            }
        }
        Ok(())
    }

    /// `ConfiguredProtoOutputFormatter.addAttributes`: the attribute values of the configured
    /// target (selects resolved).
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:349 ConfiguredProtoOutputFormatter.addAttributes
    fn add_attributes(
        &self,
        current_target: &CqueryNode,
        rule_pb: &mut RulePb,
        rule: &Arc<Rule>,
        label_printer: &LabelPrinter,
    ) {
        // We know currentTarget will be either an AliasConfiguredTarget or
        // RuleConfiguredTarget, because this method is only triggered in
        // ProtoOutputFormatter.toTargetProtoBuffer when the target in currentTarget is an
        // instanceof Rule.
        let config_conditions = get_config_conditions(current_target);
        let attribute_mapper = configured_attribute_mapper(
            rule,
            config_conditions,
            // currentTarget.getConfigurationKey().getOptionsChecksum()
            Some(
                current_target
                    .get_configuration_key()
                    .expect("NullPointerException: configuration key of a rule")
                    .get_options()
                    .checksum()
                    .to_owned(),
            ),
            /* always_succeed= */ false,
        );
        let options = self.base.options;
        let attribute_predicate = new_attribute_predicate(&options.proto_output_rule_attributes);
        for attr in sort_attributes(rule.get_attributes()) {
            // ProtoOutputFormatter.shouldIncludeAttribute
            if !((options.proto_include_default_values
                || rule.is_attribute_value_explicitly_specified_for(attr))
                && attribute_predicate(attr.get_name()))
            {
                continue;
            }
            let attribute_value = attribute_mapper.get(attr.get_name(), attr.get_type());
            let serialized_attribute = attribute_formatter::get_attribute_proto(
                attr.get_name(),
                attr.get_type(),
                attribute_value.as_ref(),
                rule.is_attribute_value_explicitly_specified_for(attr),
                /* encode_boolean_and_tri_state_as_integer_and_string= */ true,
                /* source_aspect= */ None,
                options.proto_include_attribute_source_aspects,
                label_printer,
            );
            rule_pb.attribute.push(serialized_attribute);
        }
    }
}

/// `ProtoOutputFormatter.newAttributePredicate`.
// Java: com/google/devtools/build/lib/query2/query/output/ProtoOutputFormatter.java:160 ProtoOutputFormatter.newAttributePredicate
fn new_attribute_predicate(output_attributes: &[String]) -> impl Fn(&str) -> bool + use<> {
    let all = output_attributes.len() == 1 && output_attributes[0] == "all";
    let set: HashSet<String> = output_attributes.iter().cloned().collect();
    move |name: &str| {
        if all { true } else { set.contains(name) }
    }
}

// Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:380 ProtoOutputFormatterCallback.sortAttributes
fn sort_attributes(attributes: &[Attribute]) -> Vec<&Attribute> {
    let mut sorted: Vec<&Attribute> = attributes.iter().collect();
    // Ordering.from(Comparator.comparing(Attribute::getName)).sortedCopy: a stable sort by
    // String.compareTo (attribute names are ASCII).
    sorted.sort_by(|a, b| a.get_name().cmp(b.get_name()));
    sorted
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for ProtoOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:268 ProtoOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        self.output_type.format_name().to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for ProtoOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for ProtoOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for ProtoOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:189 ProtoOutputFormatterCallback.start
    fn start(&self) -> Result<(), blaze_query_engine::callback::IoException> {
        *self.cquery_result_builder.borrow_mut() = analysis::CqueryResult::default();
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:273 ProtoOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        let options = self.base.options;
        let formatter = ProtoOutputFormatter::new(ProtoKind::Proto);
        formatter.set_options(
            &as_query_options(options),
            self.resolver.clone(),
            self.hash_function,
        );
        let converter = formatter.target_proto_converter();
        for keyed_configured_target in partial_result {
            let mut builder = analysis::ConfiguredTarget::default();
            // Re: testing. Since this formatter relies on the heavily tested ProtoOutputFormatter
            // class for all its work with targets, ProtoOutputFormatterCallbackTest doesn't test
            // any of the logic in this next line. If this were to change (i.e. we manipulate
            // targets any further), we will want to add relevant tests.
            let target = self
                .base
                .accessor
                .get_accessor_target(keyed_configured_target);
            // PORT: Java overrides ProtoOutputFormatter.addAttributes in an anonymous subclass;
            // blaze_query's TargetProtoConverter has no override hook yet. The base converter
            // adds its attributes first (one per attribute ProtoOutputFormatter.
            // shouldIncludeAttribute keeps, with the same options), so that leading block is
            // replaced by this callback's attributes.
            let mut target_builder = converter
                .to_target_proto_buffer_with_extra(&target, &self.label_printer, "")
                .map_err(OutputFormatterError::Interrupted)?;
            if let Target::Rule(rule) = &target
                && let Some(rule_pb) = target_builder.rule.as_mut()
            {
                let attribute_predicate =
                    new_attribute_predicate(&options.proto_output_rule_attributes);
                let base_attribute_count = rule
                    .get_attributes()
                    .iter()
                    .filter(|attr| {
                        (options.proto_include_default_values
                            || rule.is_attribute_value_explicitly_specified_for(attr))
                            && attribute_predicate(attr.get_name())
                    })
                    .count();
                let mut own = RulePb::default();
                self.add_attributes(keyed_configured_target, &mut own, rule, &self.label_printer);
                rule_pb
                    .attribute
                    .splice(0..base_attribute_count, own.attribute);
            }
            if matches!(target, Target::Rule(_)) && options.transitions != Transitions::NONE {
                // To set configured_rule_input dependencies, use
                // ConfiguredTargetAccessor.getPrerequisites. Note that both that and
                // CqueryTransitionResolver can get a target's direct deps. We use the former
                // because it implements cquery's "canonical" view of the dependency graph, which
                // might not match the underlying Skyframe graph.
                //
                // cquery users should always view the graph according to cquery's canonical
                // interpretation.
                for dep in self
                    .base
                    .accessor
                    .get_all_prerequisites(keyed_configured_target)
                {
                    let mut configured_rule_input = ConfiguredRuleInput {
                        label: Some(self.label_printer.to_string(&dep.get_original_label())),
                        ..Default::default()
                    };
                    if let Some(checksum) = dep.get_configuration_checksum() {
                        configured_rule_input.configuration_checksum = Some(checksum);
                        let options = dep
                            .get_configuration_key()
                            .expect("configuration key")
                            .get_options()
                            .clone();
                        let configuration = self
                            .get_configuration_for_options(&options)
                            .expect("NullPointerException: configuration");
                        configured_rule_input.configuration_id =
                            Some(self.configuration_cache.borrow_mut().get_id(&configuration));
                    }
                    target_builder
                        .rule
                        .as_mut()
                        .expect("rule target")
                        .configured_rule_input
                        .push(configured_rule_input);
                }
            }

            builder.target = Some(target_builder.clone());

            if options.proto_include_configurations {
                let checksum = keyed_configured_target.get_configuration_checksum();
                #[allow(deprecated)]
                {
                    builder.configuration = Some(analysis::Configuration {
                        // String.valueOf(checksum)
                        checksum: checksum.unwrap_or_else(|| "null".to_owned()),
                        ..Default::default()
                    });
                }

                // ConfiguredTargetKey.fromConfiguredTarget(keyedConfiguredTarget)
                // Some targets don't have a configuration, e.g. InputFileConfiguredTarget
                if let Some(configuration_key) = keyed_configured_target.get_configuration_key() {
                    let configuration = self
                        .base
                        .get_configuration(Some(&configuration_key))
                        .expect("NullPointerException: configuration");
                    let id = self.configuration_cache.borrow_mut().get_id(&configuration);
                    builder.configuration_id = id;
                }
            }

            if self.output_type == OutputType::DELIMITED_BINARY {
                // If --proto:include_configurations, we wrap the single ConfiguredTarget in a
                // CqueryResult. If --noproto:include_configurations, we wrap the single Target in
                // a QueryResult. Then we write either result delimited to the stream.
                if options.proto_include_configurations {
                    self.write_data(&analysis::CqueryResult {
                        results: vec![builder],
                        configurations: Vec::new(),
                    })?;
                } else {
                    self.write_data(&QueryResult {
                        target: vec![target_builder],
                    })?;
                }
            } else {
                // Except --output=streamed_proto, all other output types require they be wrapped
                // in a CqueryResult or QueryResult. So we instead of writing straight to the
                // stream, we aggregate the results in a CqueryResult.Builder before writing in
                // close.
                self.cquery_result_builder
                    .borrow_mut()
                    .results
                    .push(builder);
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallback.java:202 ProtoOutputFormatterCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if fail_fast || self.base.out.is_none() {
            return Ok(());
        }
        let options = self.base.options;

        // There are a few cases that affect the shape of the output:
        //   1. --output=proto|textproto|jsonproto --proto:include_configurations =>
        //        Writes a single CqueryResult containing all the ConfiguredTarget(s) and
        //        Configuration(s) in the specified output format.
        //   2. --output=streamed_proto --proto:include_configurations =>
        //        Writes multiple length delimited CqueryResult protos, each containing a single
        //        ConfiguredTarget or Configuration.
        //   3. --output=proto|textproto|jsonproto --noproto:include_configurations =>
        //        Writes a single QueryResult containing all the corresponding Target(s) in the
        //        specified output format.
        //   4.--output=streamed_proto --noproto:include_configurations =>
        //        Writes multiple length delimited QueryResult protos, each containing a single
        //        Target.
        match self.output_type {
            OutputType::BINARY | OutputType::TEXT | OutputType::JSON => {
                // Only at the end, we write the entire CqueryResult / QueryResult is written all
                // together.
                if options.proto_include_configurations {
                    let configurations = self.configuration_cache.borrow().get_configurations();
                    self.cquery_result_builder
                        .borrow_mut()
                        .configurations
                        .extend(configurations);
                }
                if options.proto_include_configurations {
                    let result = self.cquery_result_builder.borrow().clone();
                    self.write_data(&result)?;
                } else {
                    let result =
                        Self::query_result_from_cquery_result(&self.cquery_result_builder.borrow());
                    self.write_data(&result)?;
                }
            }
            OutputType::DELIMITED_BINARY => {
                if options.proto_include_configurations {
                    // The wrapped CqueryResult + ConfiguredTarget are already written in
                    // processOutput, so we just need to write the Configuration(s) each wrapped
                    // in a CqueryResult.
                    for configuration in self.configuration_cache.borrow().get_configurations() {
                        self.write_data(&analysis::CqueryResult {
                            results: Vec::new(),
                            configurations: vec![configuration],
                        })?;
                    }
                }
            }
        }

        if let Some(out) = &self.base.out {
            out.borrow_mut().flush()?;
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ProtoOutputFormatterCallbackTest.java
    const NEEDS: &str = "needs the ConfiguredTargetQueryHelper analysis harness";

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_select_in_attribute() {
        let _ = NEEDS;
    }

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_configurations() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_configuration_cpu() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_alias() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_alias_with_select() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_all_output_formats_equivalent_to_proto_output() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_all_output_formats_equivalent_to_proto_output_no_include_configurations() {}

    #[test]
    fn output_type_format_names() {
        use super::OutputType;
        assert_eq!(OutputType::BINARY.format_name(), "proto");
        assert_eq!(OutputType::DELIMITED_BINARY.format_name(), "streamed_proto");
        assert_eq!(OutputType::TEXT.format_name(), "textproto");
        assert_eq!(OutputType::JSON.format_name(), "jsonproto");
    }
}
