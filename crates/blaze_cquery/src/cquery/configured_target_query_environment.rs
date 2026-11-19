//! Ports of `query2/cquery/ConfiguredTargetQueryEnvironment.java` (the parts not shared with
//! aquery, which live in `post_analysis_query_environment`) and
//! `query2/cquery/ConfiguredTargetAccessor.java`.
//!
//! Java's interface `CqueryNode` (`query2/common/CqueryNode.java`), implemented by
//! `ConfiguredTarget` and by `AspectKey` (with `--experimental_explicit_aspects`), is the enum
//! [`CqueryNode`].

use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
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
use blaze_skyframe::{SkyKey, WalkableGraph};
use indexmap::IndexMap;

use crate::cquery::config_function::{ConfigFunction, ConfigFunctionSlot};
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::{
    PostAnalysisNode, PostAnalysisQueryEnvironment, TopLevelConfigurations,
    as_configured_target_key,
};

/// A node of the configured target graph: the information necessary to traverse the
/// different types of nodes that can be visited during a cquery call.
// Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:33 CqueryNode
#[derive(Clone)]
pub enum CqueryNode {
    ConfiguredTarget(ConfiguredTarget),
    /// An aspect node (`--experimental_explicit_aspects`): Java's `AspectKey implements
    /// CqueryNode`.
    AspectKey(AspectKey),
}

impl From<ConfiguredTarget> for CqueryNode {
    fn from(configured_target: ConfiguredTarget) -> Self {
        CqueryNode::ConfiguredTarget(configured_target)
    }
}

impl CqueryNode {
    /// Returns a key that may be used to lookup this `CqueryNode`.
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:33 CqueryNode.getLookupKey
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:182 AspectKeyCreator.AspectKey.getLookupKey
    pub fn get_lookup_key(&self) -> ActionLookupKeyRef {
        match self {
            CqueryNode::ConfiguredTarget(ct) => ct.get_lookup_key().clone(),
            CqueryNode::AspectKey(key) => ActionLookupKeyRef::new(key.clone()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:35 CqueryNode.getLabel
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:172 AspectKeyCreator.AspectKey.getLabel
    pub fn get_label(&self) -> Label {
        match self {
            CqueryNode::ConfiguredTarget(ct) => ct.get_label(),
            CqueryNode::AspectKey(key) => key.get_label().clone(),
        }
    }

    /// The text cquery prints for the node: the original label for a configured target, the
    /// aspect's description (`<aspect class> of <label>`) for an aspect.
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:39 CqueryNode.getDescription
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:152 AspectKeyCreator.AspectKey.getDescription
    pub fn get_description(&self, label_printer: &LabelPrinter) -> String {
        match self {
            CqueryNode::ConfiguredTarget(ct) => label_printer.to_string(&ct.get_original_label()),
            CqueryNode::AspectKey(key) => key.get_description(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:44 CqueryNode.getConfigurationChecksum
    pub fn get_configuration_checksum(&self) -> Option<String> {
        self.get_configuration_key()
            .map(|k| k.get_options().checksum().to_owned())
    }

    /// The key of the configuration the node is defined for (`None` for input files and package
    /// groups; an aspect's is its base configured target's).
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:59 CqueryNode.getConfigurationKey
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:205 AspectKeyCreator.AspectKey.getConfigurationKey
    pub fn get_configuration_key(&self) -> Option<BuildConfigurationKey> {
        match self {
            CqueryNode::ConfiguredTarget(ct) => ct.get_configuration_key(),
            CqueryNode::AspectKey(key) => key.get_configuration_key().cloned(),
        }
    }

    /// If the configured target is an alias, return the actual target, otherwise return the
    /// current target. This follows alias chains.
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:67 CqueryNode.getActual
    pub fn get_actual(&self) -> CqueryNode {
        match self {
            CqueryNode::ConfiguredTarget(ct) => CqueryNode::ConfiguredTarget(ct.get_actual()),
            CqueryNode::AspectKey(_) => self.clone(),
        }
    }

    /// If the configured target is an alias, return the original label, otherwise return the
    /// current label. This is not the same as `get_actual().get_label()`, because it does not
    /// follow alias chains.
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:76 CqueryNode.getOriginalLabel
    pub fn get_original_label(&self) -> Label {
        match self {
            CqueryNode::ConfiguredTarget(ct) => ct.get_original_label(),
            CqueryNode::AspectKey(_) => self.get_label(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:88 CqueryNode.isRuleConfiguredTarget
    pub fn is_rule_configured_target(&self) -> bool {
        match self {
            CqueryNode::ConfiguredTarget(ct) => ct.is_rule_configured_target(),
            CqueryNode::AspectKey(_) => false,
        }
    }

    /// The base configured target if it has been merged with aspects otherwise the current
    /// value. Unwrapping is recursive if there are multiple layers.
    // Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:97 CqueryNode.unwrapIfMerged
    pub fn unwrap_if_merged(&self) -> CqueryNode {
        match self {
            CqueryNode::ConfiguredTarget(ct) => CqueryNode::ConfiguredTarget(ct.unwrap_if_merged()),
            CqueryNode::AspectKey(_) => self.clone(),
        }
    }

    /// The configured target of the node (`node instanceof ConfiguredTarget`).
    pub fn as_configured_target(&self) -> Option<&ConfiguredTarget> {
        match self {
            CqueryNode::ConfiguredTarget(ct) => Some(ct),
            CqueryNode::AspectKey(_) => None,
        }
    }
}

impl PostAnalysisNode for CqueryNode {
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:560 ConfiguredTargetQueryEnvironment.getConfiguredTargetKey
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        CqueryNode::get_lookup_key(self)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:549 ConfiguredTargetQueryEnvironment.getConfiguration
    fn get_configuration_key(&self) -> Option<BuildConfigurationKey> {
        CqueryNode::get_configuration_key(self)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:494 ConfiguredTargetQueryEnvironment.getCorrectLabel
    fn get_original_label(&self) -> Label {
        CqueryNode::get_original_label(self)
    }

    fn as_configured_target(&self) -> Option<&ConfiguredTarget> {
        CqueryNode::as_configured_target(self)
    }

    /// Returns the `CqueryNode` for the given key if its value is a supported instance of
    /// CqueryNode. This function can only receive keys of node types that the calling logic can
    /// support. For example, if the caller does not support handling of AspectKey types of
    /// CqueryNodes, then this function should not be called with an AspectKey key.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:327 ConfiguredTargetQueryEnvironment.getValueFromKey
    fn get_value_from_key(graph: &dyn WalkableGraph, key: &SkyKey) -> Option<Self> {
        let value = graph.get_value(key)?;
        if let Ok(configured_target_value) = value.clone().downcast::<ConfiguredTargetValue>() {
            return Some(CqueryNode::ConfiguredTarget(
                configured_target_value.get_configured_target().clone(),
            ));
        }
        // The value is intentionally ignored as the key implements CqueryNode.
        if value.is::<AspectValue>()
            && let Some(aspect_key) = key.downcast::<AspectKey>()
        {
            return Some(CqueryNode::AspectKey(aspect_key.clone()));
        }
        panic!("IllegalStateException: unknown value type for CqueryNode")
    }

    fn is_instance_of_rule_configured_target(&self) -> bool {
        matches!(
            self,
            CqueryNode::ConfiguredTarget(ConfiguredTarget::Rule(_))
        )
    }
}

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
    top_level_aspects: &IndexMap<AspectKey, Arc<AspectValue>>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
    walkable_graph: Arc<dyn WalkableGraph>,
    cquery_options: &CqueryOptions,
    label_printer: LabelPrinter,
) -> ConfiguredTargetQueryEnvironment {
    let mut env = PostAnalysisQueryEnvironment::new(
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
    );
    // this.accessor = new ConfiguredTargetAccessor(walkableGraphSupplier.get(), this,
    //     topLevelAspects);
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:73 ConfiguredTargetAccessor.<init>
    // PORT: Java memoizes the multimap (Suppliers.memoize); it is built eagerly here, which is
    // not observable.
    if env
        .settings
        .contains(&blaze_query_engine::Setting::IncludeAspects)
    {
        // topLevelAspects.entrySet().stream().collect(toImmutableListMultimap(
        //     entry -> entry.getKey().getBaseConfiguredTargetKey(), Map.Entry::getValue))
        for (key, value) in top_level_aspects {
            env.top_level_aspects_by_target
                .entry(key.get_base_configured_target_key().clone())
                .or_default()
                // PORT: AnalysisResult.getAspectsMap() holds the AspectValues (each a
                // ConfiguredAspect in Java).
                .push(ConfiguredAspect::Value(value.clone()));
        }
    }
    env
}

/// `CqueryNode.getConfigConditions()`: the `config_setting`s a rule's `select()`s read.
// Java: com/google/devtools/build/lib/query2/common/CqueryNode.java:84 CqueryNode.getConfigConditions
pub fn get_config_conditions(target: &CqueryNode) -> Arc<IndexMap<Label, ConfigMatchingProvider>> {
    match target {
        CqueryNode::ConfiguredTarget(ConfiguredTarget::Rule(r)) => {
            r.get_config_conditions().clone()
        }
        CqueryNode::ConfiguredTarget(ConfiguredTarget::Alias(a)) => {
            a.get_config_conditions().clone()
        }
        // Java: CqueryNode.getConfigConditions() default (ImmutableMap.of()).
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
    /// Returns the top-level aspects applied to the given `CqueryNode`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetAccessor.java:294 ConfiguredTargetAccessor.getTopLevelAspects
    pub fn get_top_level_aspects(&self, cn: &CqueryNode) -> Vec<ConfiguredAspect> {
        let lookup_key = cn.get_lookup_key();
        let Some(key) = as_configured_target_key(&lookup_key) else {
            return Vec::new();
        };
        self.top_level_aspects_by_target
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

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

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:767 ConfiguredTargetQuerySemanticsTest.testAspectDepsAppearInCqueryDeps
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn test_aspect_deps_appear_in_cquery_deps() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:839 ConfiguredTargetQuerySemanticsTest.testToolchainPropagatingAspectDepsAppearInCqueryDeps
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn test_toolchain_propagating_aspect_deps_appear_in_cquery_deps() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:949 ConfiguredTargetQuerySemanticsTest.testAspectOnAspectDepsAppearInCqueryDeps
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn test_aspect_on_aspect_deps_appear_in_cquery_deps() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:1078 ConfiguredTargetQuerySemanticsTest.testAspectDepsAppearInCqueryRdeps
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn test_aspect_deps_appear_in_cquery_rdeps() {}
}
