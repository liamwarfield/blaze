//! Ports of `query2/cquery/ConfiguredTargetQueryEnvironment.java` (the parts not shared with
//! aquery, which live in `post_analysis_query_environment`) and
//! `query2/cquery/ConfiguredTargetAccessor.java`.
//!
//! PORT: Java's `CqueryNode` is implemented by `ConfiguredTarget` and by `AspectKey` (with
//! `--experimental_explicit_aspects`); aspect nodes are not ported, so a node is a
//! `ConfiguredTarget`.

use std::sync::Arc;

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::{Label, TargetPatternParser};
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::target::target_utils;
use blaze_packages::target::{LabelPrinter, Rule, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::failure_details::configurable_query;
use blaze_query_engine::{
    QueryError, QueryException, QueryExpression, QueryFunction, QueryVisibility, TargetAccessor,
    default_query_functions,
};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_skyframe::WalkableGraph;
use indexmap::IndexMap;

use crate::cquery::config_function::{ConfigFunction, ConfigFunctionSlot};
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::{
    PostAnalysisQueryEnvironment, TopLevelConfigurations,
};

/// A node of the configured target graph (Java `CqueryNode`).
pub type CqueryNode = ConfiguredTarget;

/// `QueryEnvironment` that runs queries over the configured target (analysis) graph.
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:80 ConfiguredTargetQueryEnvironment
pub type ConfiguredTargetQueryEnvironment = PostAnalysisQueryEnvironment<CqueryNode>;

/// Cquery specific functions.
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:170 ConfiguredTargetQueryEnvironment.getCqueryFunctions
pub fn cquery_functions(slot: &ConfigFunctionSlot) -> Vec<Arc<dyn QueryFunction<CqueryNode>>> {
    vec![Arc::new(ConfigFunction::new(slot.clone()))]
}

/// Common query functions and cquery specific functions.
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:163 ConfiguredTargetQueryEnvironment.populateFunctions
pub fn functions(slot: &ConfigFunctionSlot) -> Vec<Arc<dyn QueryFunction<CqueryNode>>> {
    let mut functions = default_query_functions::<CqueryNode>();
    functions.extend(cquery_functions(slot));
    functions
}

/// Creates the environment (Java `new ConfiguredTargetQueryEnvironment(..., cqueryOptions, ...)`).
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:129 ConfiguredTargetQueryEnvironment.<init>
pub fn new_configured_target_query_environment(
    keep_going: bool,
    event_handler: Arc<dyn ExtendedEventHandler>,
    extra_functions: Vec<Arc<dyn QueryFunction<CqueryNode>>>,
    top_level_configurations: TopLevelConfigurations,
    transitive_configurations: IndexMap<String, Arc<BuildConfigurationValue>>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
    walkable_graph: Arc<dyn WalkableGraph>,
    cquery_options: &CqueryOptions,
    label_printer: LabelPrinter,
) -> ConfiguredTargetQueryEnvironment {
    // PORT: top-level aspects (ConfiguredTargetAccessor.topLevelAspectsByTarget) are not
    // ported; --aspects is rejected by BuildView.
    PostAnalysisQueryEnvironment::new(
        keep_going,
        event_handler,
        extra_functions,
        top_level_configurations,
        transitive_configurations,
        main_repo_target_parser,
        pkg_path,
        walkable_graph,
        cquery_options.to_settings(),
        label_printer,
    )
}

/// `CqueryNode.getConfigConditions()`: the `config_setting`s a rule's `select()`s read.
pub fn get_config_conditions(
    target: &ConfiguredTarget,
) -> Arc<IndexMap<Label, ConfigMatchingProvider>> {
    match target {
        ConfiguredTarget::Rule(r) => r.get_config_conditions().clone(),
        ConfiguredTarget::Alias(a) => a.get_config_conditions().clone(),
        // Java: ConfiguredTarget.getConfigConditions() default (ImmutableMap.of()).
        _ => Arc::new(IndexMap::new()),
    }
}

/// `ConfiguredAttributeMapper.of(rule, configConditions, configHash, alwaysSucceed)`.
pub fn configured_attribute_mapper(
    rule: &Arc<Rule>,
    config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
    config_hash: Option<String>,
    always_succeed: bool,
) -> ConfiguredAttributeMapper {
    ConfiguredAttributeMapper::of_hash(
        rule.clone(),
        config_conditions,
        // String.valueOf(null)
        config_hash.unwrap_or_else(|| "null".to_owned()),
        always_succeed,
    )
}

impl ConfiguredTargetQueryEnvironment {
    /// The target of a configured target, dereferencing aliases.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:278 ConfiguredTargetAccessor.getTarget
    pub fn get_accessor_target(&self, configured_target: &CqueryNode) -> Target {
        // Dereference any aliases that might be present.
        let label = configured_target.get_original_label();
        self.get_target_or_panic(&label)
    }

    /// Returns all of `keyed_configured_target`'s prerequisites (cquery's canonical direct
    /// dependencies).
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:158 ConfiguredTargetAccessor.getPrerequisites
    pub fn get_all_prerequisites(&self, keyed_configured_target: &CqueryNode) -> Vec<CqueryNode> {
        self.get_fwd_deps_set(std::slice::from_ref(keyed_configured_target))
            .to_vec()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:250 ConfiguredTargetAccessor.getAttributes
    fn with_attributes<R>(
        &self,
        target: &CqueryNode,
        f: impl FnOnce(&ConfiguredAttributeMapper, &Arc<Rule>) -> R,
    ) -> R {
        let actual_target = self.get_accessor_target(target);
        let Target::Rule(rule) = &actual_target else {
            panic!("IllegalArgumentException");
        };
        let attribute_mapper = configured_attribute_mapper(
            rule,
            get_config_conditions(target),
            target.get_configuration_checksum(),
            /* always_succeed= */ false,
        );
        f(&attribute_mapper, rule)
    }
}

impl TargetAccessor<CqueryNode> for ConfiguredTargetQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:104 ConfiguredTargetAccessor.getTargetKind
    fn get_target_kind(&self, target: &CqueryNode) -> String {
        self.get_accessor_target(target).get_target_kind()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:110 ConfiguredTargetAccessor.getLabel
    fn get_label(&self, target: &CqueryNode) -> String {
        target.get_original_label().to_string()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:115 ConfiguredTargetAccessor.getPackage
    fn get_package(&self, target: &CqueryNode) -> String {
        target
            .get_original_label()
            .get_package_identifier()
            .get_package_fragment()
            .get_path_string()
            .to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:120 ConfiguredTargetAccessor.isRule
    fn is_rule(&self, target: &CqueryNode) -> bool {
        matches!(self.get_accessor_target(target), Target::Rule(_))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:126 ConfiguredTargetAccessor.isExecutableNonTestRule
    fn is_executable_non_test_rule(&self, target: &CqueryNode) -> bool {
        target_utils::is_executable_non_test_rule(&self.get_accessor_target(target))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:132 ConfiguredTargetAccessor.isTestRule
    fn is_test_rule(&self, target: &CqueryNode) -> bool {
        target_utils::is_test_rule(&self.get_accessor_target(target))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:138 ConfiguredTargetAccessor.isTestSuite
    fn is_test_suite(&self, target: &CqueryNode) -> bool {
        target_utils::is_test_suite_rule(&self.get_accessor_target(target))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:163 ConfiguredTargetAccessor.getPrerequisites
    fn get_prerequisites(
        &self,
        caller: &QueryExpression,
        keyed_configured_target: &CqueryNode,
        attr_name: &str,
        error_msg_prefix: &str,
    ) -> Result<Vec<CqueryNode>, QueryError> {
        // Process aliases.
        let actual = keyed_configured_target.get_actual();

        if !self.is_rule(&actual) {
            panic!(
                "IllegalArgumentException: {} {} is not a rule configured target",
                error_msg_prefix,
                TargetAccessor::get_label(self, &actual)
            );
        }

        // Multimaps.index(getFwdDeps([actual]), CqueryNode::getOriginalLabel)
        let mut deps_by_label: IndexMap<Label, Vec<CqueryNode>> = IndexMap::new();
        for dep in self.get_all_prerequisites(&actual) {
            deps_by_label
                .entry(dep.get_original_label())
                .or_default()
                .push(dep);
        }

        let Target::Rule(rule) = self.get_accessor_target(&actual) else {
            panic!("ClassCastException: not a Rule");
        };
        let attribute_mapper = configured_attribute_mapper(
            &rule,
            get_config_conditions(&actual),
            keyed_configured_target.get_configuration_checksum(),
            /* always_succeed= */ false,
        );
        if !attribute_mapper.has(attr_name) {
            return Err(QueryException::with_configurable_query_code(
                Some(caller),
                format!(
                    "{}configured target of type {} does not have attribute '{}'",
                    error_msg_prefix,
                    rule.get_rule_class(),
                    attr_name
                ),
                configurable_query::Code::AttributeMissing,
            )
            .into());
        }
        let mut to_return: Vec<CqueryNode> = Vec::new();
        attribute_mapper.visit_labels(attr_name, &mut |label: &Label| {
            if let Some(deps) = deps_by_label.get(label) {
                to_return.extend(deps.iter().cloned());
            }
        });
        Ok(to_return)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:200 ConfiguredTargetAccessor.getStringListAttr
    fn get_string_list_attr(&self, target: &CqueryNode, attr_name: &str) -> Vec<String> {
        self.with_attributes(target, |attribute_mapper, _| {
            match attribute_mapper.get(attr_name, Type::StringList) {
                Some(AttrValue::StringList(l)) => l,
                other => panic!("ClassCastException: {attr_name} value {other:?}"),
            }
        })
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:206 ConfiguredTargetAccessor.getStringAttr
    fn get_string_attr(&self, target: &CqueryNode, attr_name: &str) -> Option<String> {
        self.with_attributes(target, |attribute_mapper, _| {
            attribute_mapper
                .get(attr_name, Type::String)
                .map(|v| match v {
                    AttrValue::String(s) => s,
                    other => panic!("ClassCastException: {attr_name} value {other:?}"),
                })
        })
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:212 ConfiguredTargetAccessor.getAttrAsString
    fn get_attr_as_string(&self, target: &CqueryNode, attr_name: &str) -> Vec<String> {
        self.with_attributes(target, |attribute_mapper, rule| {
            let Some(attribute) = attribute_mapper.get_attribute_definition(attr_name) else {
                // Ignore unknown attributes.
                return Vec::new();
            };
            let attribute_type = attribute.get_type();

            let Some(mut value) = attribute_mapper.get(attr_name, attribute_type) else {
                return Vec::new();
            };

            if attr_name == "visibility" && attribute_type == Type::NodepLabelList {
                // This special case for the visibility attribute is needed because its value is
                // replaced with an empty list during package loading if it is public or private
                // in order not to visit the package called 'visibility'.
                value = AttrValue::LabelList(
                    Target::Rule(rule.clone()).get_visibility_declared_labels(),
                );
            }

            // Return a single-valued list, because a configured target only has one value for
            // the attribute. Flatten to a string regardless of the actual type so that
            // regex-based matches can be performed.
            target_utils::convert_attribute_value(attribute_type, Some(&value))
                .into_iter()
                .collect()
        })
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:242 ConfiguredTargetAccessor.getVisibility
    fn get_visibility<'a>(
        &'a self,
        _caller: &QueryExpression,
        _from: &CqueryNode,
    ) -> Result<Vec<Arc<dyn QueryVisibility<CqueryNode> + 'a>>, QueryError> {
        // TODO(bazel-team): implement this if needed.
        Err(QueryException::with_configurable_query_code(
            None,
            "visible() is not supported on configured targets",
            configurable_query::Code::VisibleFunctionNotSupported,
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryTest.java
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_multiple_top_level_configurations_null_configs() {}

    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_implicit_toolchain_binding_contains_toolchain_target() {}
}
