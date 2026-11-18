//! Ports of `query2/aquery/ActionGraphQueryEnvironment.java` (the parts not shared with cquery,
//! which live in `post_analysis_query_environment`) and
//! `query2/aquery/ConfiguredTargetValueAccessor.java`.

use std::sync::Arc;

use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::TargetPatternParser;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{action_query, configurable_query};
use blaze_query::query::blaze_target_accessor;
use blaze_query_engine::{
    InputsFunction, MnemonicFunction, OutputsFunction, QueryError, QueryException, QueryExpression,
    QueryFunction, QueryVisibility, TargetAccessor, default_query_functions,
};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_skyframe::WalkableGraph;
use indexmap::IndexMap;

use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::{
    PostAnalysisQueryEnvironment, TopLevelConfigurations,
};

/// A node of the action graph query (Java `ConfiguredTargetValue`).
pub type KeyedConfiguredTargetValue = Arc<ConfiguredTargetValue>;

/// `QueryEnvironment` over the action graph.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:70 ActionGraphQueryEnvironment
pub type ActionGraphQueryEnvironment = PostAnalysisQueryEnvironment<KeyedConfiguredTargetValue>;

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:139 ActionGraphQueryEnvironment.populateAqueryFunctions
pub fn aquery_functions() -> Vec<Arc<dyn QueryFunction<KeyedConfiguredTargetValue>>> {
    vec![
        Arc::new(InputsFunction::new()),
        Arc::new(OutputsFunction::new()),
        Arc::new(MnemonicFunction::new()),
    ]
}

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:135 ActionGraphQueryEnvironment.populateFunctions
pub fn functions() -> Vec<Arc<dyn QueryFunction<KeyedConfiguredTargetValue>>> {
    default_query_functions::<KeyedConfiguredTargetValue>()
}

/// Creates the environment (Java `new ActionGraphQueryEnvironment(..., aqueryOptions, ...)`).
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:108 ActionGraphQueryEnvironment.<init>
pub fn new_action_graph_query_environment(
    keep_going: bool,
    event_handler: Arc<dyn ExtendedEventHandler>,
    extra_functions: Vec<Arc<dyn QueryFunction<KeyedConfiguredTargetValue>>>,
    top_level_configurations: TopLevelConfigurations,
    transitive_configurations: IndexMap<String, Arc<BuildConfigurationValue>>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
    walkable_graph: Arc<dyn WalkableGraph>,
    aquery_options: &AqueryOptions,
    label_printer: LabelPrinter,
) -> ActionGraphQueryEnvironment {
    PostAnalysisQueryEnvironment::new(
        keep_going,
        event_handler,
        extra_functions,
        top_level_configurations,
        transitive_configurations,
        main_repo_target_parser,
        pkg_path,
        walkable_graph,
        aquery_options.to_settings(),
        label_printer,
    )
}

impl ActionGraphQueryEnvironment {
    /// The target of a value, dereferencing aliases.
    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:151 ConfiguredTargetValueAccessor.getTargetFromConfiguredTargetValue
    pub fn get_target_from_configured_target_value(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
    ) -> Target {
        // Dereference any aliases that might be present.
        let label = configured_target_value
            .get_configured_target()
            .get_original_label();
        self.get_target_or_panic(&label)
    }

    /// Returns the AspectValues that are attached to the given configuredTarget.
    ///
    /// PORT: `AspectValue` (AspectFunction) is not ported (blaze_sky_analysis::aspect_function),
    /// so the result is empty unless an `ASPECT` node depends on the target, which fails loudly.
    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:163 ConfiguredTargetValueAccessor.getAspectValues
    pub fn get_aspect_values(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
    ) -> Vec<std::convert::Infallible> {
        let sky_key = crate::post_analysis_query_environment::lookup_key_to_sky_key(
            &crate::post_analysis_query_environment::get_configured_target_key(
                configured_target_value,
            ),
        );
        let label = configured_target_value.get_configured_target().get_label();
        for (_, rev_deps) in self.get_graph().get_reverse_deps(&[sky_key]) {
            for rev_dep in rev_deps {
                if rev_dep.function_name().0 == "ASPECT" {
                    blaze_util::unported!(
                        "ConfiguredTargetValueAccessor.getAspectValues",
                        "AspectValue of {:?} on {}",
                        rev_dep,
                        label
                    );
                }
            }
        }
        Vec::new()
    }
}

impl TargetAccessor<KeyedConfiguredTargetValue> for ActionGraphQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:63 ConfiguredTargetValueAccessor.getTargetKind
    fn get_target_kind(&self, configured_target_value: &KeyedConfiguredTargetValue) -> String {
        self.get_target_from_configured_target_value(configured_target_value)
            .get_target_kind()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:69 ConfiguredTargetValueAccessor.getLabel
    fn get_label(&self, configured_target_value: &KeyedConfiguredTargetValue) -> String {
        configured_target_value
            .get_configured_target()
            .get_label()
            .to_string()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:74 ConfiguredTargetValueAccessor.getPackage
    fn get_package(&self, configured_target_value: &KeyedConfiguredTargetValue) -> String {
        configured_target_value
            .get_configured_target()
            .get_label()
            .get_package_identifier()
            .get_package_fragment()
            .get_path_string()
            .to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:84 ConfiguredTargetValueAccessor.isRule
    fn is_rule(&self, configured_target_value: &KeyedConfiguredTargetValue) -> bool {
        matches!(
            self.get_target_from_configured_target_value(configured_target_value),
            Target::Rule(_)
        )
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:90 ConfiguredTargetValueAccessor.isExecutableNonTestRule
    fn is_executable_non_test_rule(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
    ) -> bool {
        blaze_packages::target::target_utils::is_executable_non_test_rule(
            &self.get_target_from_configured_target_value(configured_target_value),
        )
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:96 ConfiguredTargetValueAccessor.isTestRule
    fn is_test_rule(&self, configured_target_value: &KeyedConfiguredTargetValue) -> bool {
        blaze_packages::target::target_utils::is_test_rule(
            &self.get_target_from_configured_target_value(configured_target_value),
        )
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:102 ConfiguredTargetValueAccessor.isTestSuite
    fn is_test_suite(&self, configured_target_value: &KeyedConfiguredTargetValue) -> bool {
        blaze_packages::target::target_utils::is_test_suite_rule(
            &self.get_target_from_configured_target_value(configured_target_value),
        )
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:108 ConfiguredTargetValueAccessor.getPrerequisites
    fn get_prerequisites(
        &self,
        _caller: &QueryExpression,
        _configured_target_value: &KeyedConfiguredTargetValue,
        _attr_name: &str,
        _error_msg_prefix: &str,
    ) -> Result<Vec<KeyedConfiguredTargetValue>, QueryError> {
        // TODO(bazel-team): implement this if needed.
        Err(QueryException::with_action_query_code(
            None,
            "labels() is not supported for aquery",
            action_query::Code::LabelsFunctionNotSupported,
        )
        .into())
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:120 ConfiguredTargetValueAccessor.getStringListAttr
    fn get_string_list_attr(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
        attr_name: &str,
    ) -> Vec<String> {
        let actual_target = self.get_target_from_configured_target_value(configured_target_value);
        blaze_target_accessor::get_string_list_attr(&actual_target, attr_name)
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:127 ConfiguredTargetValueAccessor.getStringAttr
    fn get_string_attr(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
        attr_name: &str,
    ) -> Option<String> {
        let actual_target = self.get_target_from_configured_target_value(configured_target_value);
        blaze_target_accessor::get_string_attr(&actual_target, attr_name)
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:133 ConfiguredTargetValueAccessor.getAttrAsString
    fn get_attr_as_string(
        &self,
        configured_target_value: &KeyedConfiguredTargetValue,
        attr_name: &str,
    ) -> Vec<String> {
        let actual_target = self.get_target_from_configured_target_value(configured_target_value);
        blaze_target_accessor::get_attr_as_string(&actual_target, attr_name)
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ConfiguredTargetValueAccessor.java:140 ConfiguredTargetValueAccessor.getVisibility
    fn get_visibility<'a>(
        &'a self,
        _caller: &QueryExpression,
        _from: &KeyedConfiguredTargetValue,
    ) -> Result<Vec<Arc<dyn QueryVisibility<KeyedConfiguredTargetValue> + 'a>>, QueryError> {
        // TODO(bazel-team): implement this if needed.
        Err(QueryException::with_configurable_query_code(
            None,
            "visible() is not supported on configured targets",
            configurable_query::Code::VisibleFunctionNotSupported,
        )
        .into())
    }
}
