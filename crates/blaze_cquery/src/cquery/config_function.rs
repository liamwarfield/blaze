//! Port of `query2/cquery/ConfigFunction.java` and
//! `ConfiguredTargetQueryEnvironment.getConfiguredTargetsForConfigFunction`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_protos::failure_details::configurable_query;
use blaze_query_engine::query_util;
use blaze_query_engine::{
    Argument, ArgumentType, Callback, QueryEnvironment, QueryEnvironmentFutures, QueryError,
    QueryException, QueryExpression, QueryExpressionContext, QueryFunction, QueryFunctionBase,
    QueryTaskFuture, ThreadSafeMutableSet,
};
use parking_lot::RwLock;

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::post_analysis_query_environment::PostAnalysisGraph;

/// The environment `ConfigFunction` evaluates in.
///
/// PORT: Java casts the `QueryEnvironment` argument to `ConfiguredTargetQueryEnvironment`; the
/// engine's environments are not downcastable, so the cquery processor fills this slot with the
/// environment's graph lookups before evaluating (the function instances are created with the
/// command's function map, before the environment exists).
pub type ConfigFunctionSlot = Arc<RwLock<Option<PostAnalysisGraph>>>;

/// A "config" query expression for cquery. The first argument is the expression to be evaluated.
/// The second argument is "target", "null", or an arbitrary configuration's hash (the same hash
/// cquery annotates label outputs with) to specify which configuration the user is seeking to
/// query in. If some but not all results of expr can be found in the specified config, the subset
/// that can be is returned. If no results of expr can be found in the specified config, an error
/// is thrown.
///
/// `expr ::= CONFIG '(' expr ',' word ')'`
// Java: com/google/devtools/build/lib/query2/cquery/ConfigFunction.java:40 ConfigFunction
pub struct ConfigFunction {
    environment: ConfigFunctionSlot,
}

impl ConfigFunction {
    pub fn new(environment: ConfigFunctionSlot) -> Self {
        ConfigFunction { environment }
    }
}

impl QueryFunctionBase for ConfigFunction {
    // Java: com/google/devtools/build/lib/query2/cquery/ConfigFunction.java:46 ConfigFunction.getName
    fn get_name(&self) -> String {
        "config".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfigFunction.java:51 ConfigFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        2
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfigFunction.java:56 ConfigFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression, ArgumentType::Word]
    }
}

impl QueryFunction<CqueryNode> for ConfigFunction {
    /// This function is only viable with ConfiguredTargetQueryEnvironment.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfigFunction.java:65 ConfigFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<CqueryNode>,
        context: &QueryExpressionContext<CqueryNode>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<CqueryNode>,
    ) -> QueryTaskFuture<()> {
        let target_expression = &args[0];
        let mut configuration = args[1].to_string();
        // Turn "'string'" to "string" (remove the surrounding apostrophes).
        configuration = configuration[1..configuration.len() - 1].to_owned();

        let targets_future = query_util::eval_all(env, context, target_expression.get_expression());

        let lookup = self
            .environment
            .read()
            .clone()
            .expect("ClassCastException: config() outside a ConfiguredTargetQueryEnvironment");
        env.when_succeeds_call(&targets_future, || {
            get_configured_targets_for_config_function(
                &lookup,
                &target_expression.to_string(),
                &**targets_future.get_if_successful(),
                &configuration,
                callback,
            )
        })
    }
}

/// Processes the targets in `targets` with the requested `config_prefix`.
///
/// - `pattern`: the original pattern that `targets` were parsed from. Used for error message.
/// - `targets`: the set of configured targets whose labels represent the targets being
///   requested.
/// - `config_prefix`: the configuration to request `targets` in. This can be the
///   configuration's checksum, any prefix of its checksum, or the special identifiers "target"
///   or "null".
/// - `callback`: the callback to receive the results of this method.
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:409 ConfiguredTargetQueryEnvironment.getConfiguredTargetsForConfigFunction
fn get_configured_targets_for_config_function(
    env: &PostAnalysisGraph,
    pattern: &str,
    targets: &dyn ThreadSafeMutableSet<CqueryNode>,
    config_prefix: &str,
    callback: &dyn Callback<CqueryNode>,
) -> Result<(), QueryError> {
    // There's no technical reason other callers beside ConfigFunction can't call this. But
    // they'd need to adjust the error messaging below to not make it config()-specific.
    let mut transformed_result: Vec<CqueryNode> = Vec::new();
    let mut user_friendly_config_name = true;
    for target in targets.to_vec() {
        let label: Label = target.get_original_label();
        let keyed_configured_target: Option<CqueryNode> = match config_prefix {
            "host" => {
                return Err(QueryException::with_configurable_query_code(
                    None,
                    "'host' configuration no longer exists. Use a specific configuration hash \
                     instead",
                    configurable_query::Code::IncorrectConfigArgumentError,
                )
                .into());
            }
            "target" => env.get_target_configured_target(&label),
            "null" => env.get_null_configured_target(&label),
            _ => {
                let matching_configs: Vec<&String> = env
                    .transitive_configurations
                    .keys()
                    .filter(|full_config| full_config.starts_with(config_prefix))
                    .collect();
                if matching_configs.len() == 1 {
                    user_friendly_config_name = false;
                    env.get_configured_target(
                        &label,
                        Some(
                            env.transitive_configurations
                                .get(matching_configs[0])
                                .expect("verifyNotNull"),
                        ),
                    )
                } else if matching_configs.len() >= 2 {
                    return Err(QueryException::with_configurable_query_code(
                        None,
                        format!(
                            "Configuration ID '{}' is ambiguous.\n'{}' is a prefix of multiple \
                             configurations:\n {}\n\nUse a longer prefix to uniquely identify one \
                             configuration.",
                            config_prefix,
                            config_prefix,
                            matching_configs
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join("\n ")
                        ),
                        configurable_query::Code::IncorrectConfigArgumentError,
                    )
                    .into());
                } else {
                    return Err(QueryException::with_configurable_query_code(
                        None,
                        format!(
                            "Unknown configuration ID '{}'.\nconfig()'s second argument must \
                             identify a unique configuration.\n\nValid values:\n 'target' for the \
                             default configuration\n 'null' for source files (which have no \
                             configuration)\n an arbitrary configuration's full or short ID\n\nA \
                             short ID is any prefix of a full ID. cquery shows short IDs. 'bazel \
                             config' shows full IDs.\n\nFor more help, see \
                             https://bazel.build/docs/cquery.",
                            config_prefix
                        ),
                        configurable_query::Code::IncorrectConfigArgumentError,
                    )
                    .into());
                }
            }
        };
        if let Some(t) = keyed_configured_target {
            transformed_result.push(t);
        }
    }
    if transformed_result.is_empty() {
        return Err(QueryException::with_configurable_query_code(
            None,
            format!(
                "No target (in) {} could be found in the {}",
                pattern,
                if user_friendly_config_name {
                    format!("'{}' configuration", config_prefix)
                } else {
                    format!("configuration with checksum '{}'", config_prefix)
                }
            ),
            configurable_query::Code::TargetMissing,
        )
        .into());
    }
    callback.process(&transformed_result)
}
