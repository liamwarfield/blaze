//! Ports of `analysis/producers/TransitionApplier.java`,
//! `analysis/producers/BuildConfigurationKeyMapProducer.java` and the analysis side of
//! `analysis/producers/BuildConfigurationKeyProducer.java`.
//!
//! PORT: the state machines become straight-line functions over the blocking evaluator.
//! `Ok(None)` is "a Skyframe restart is needed" (a dependency is missing).

use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::starlark_transition::{
    StarlarkBuildSettingsDetailsKey, StarlarkBuildSettingsDetailsValue, StarlarkTransitionCache,
    TransitionException, command_line_flag_aliases, does_starlark_transition,
};
use blaze_analysis::config::transitions::{ConfigurationTransition, restrict};
use blaze_build_config::build_configuration_key::{
    BuildConfigurationKey, BuildConfigurationKeyValueKey,
};
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::skyframe::ConfigurationError;
use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_skyframe::{Environment, Interrupted, SkyKey};
use indexmap::IndexMap;

/// The errors `TransitionApplier.ResultSink` (and `BuildConfigurationKeyMapProducer.ResultSink`)
/// accept.
#[derive(Clone, Debug)]
pub enum TransitionApplierError {
    /// `acceptTransitionError(TransitionException)`.
    Transition(TransitionException),
    /// `acceptOptionsParsingError(OptionsParsingException)` (message, invalid argument).
    OptionsParsing(String),
    /// `acceptPlatformMappingError(PlatformMappingException)`.
    PlatformMapping(String),
    /// `acceptPlatformFlagsError(InvalidPlatformException)`.
    PlatformFlags(String),
    /// `acceptBuildOptionsScopeFunctionError(BuildOptionsScopeFunctionException)`.
    BuildOptionsScope(String),
}

impl TransitionApplierError {
    pub fn get_message(&self) -> String {
        match self {
            TransitionApplierError::Transition(e) => e.0.clone(),
            TransitionApplierError::OptionsParsing(m)
            | TransitionApplierError::PlatformMapping(m)
            | TransitionApplierError::PlatformFlags(m)
            | TransitionApplierError::BuildOptionsScope(m) => m.clone(),
        }
    }
}

/// The outcome of a producer: `Ok(None)` when a dependency is missing (Skyframe restart).
pub type Produced<T> = Result<Option<T>, Interrupted>;

/// Applies `transition` to `from_configuration` and maps each resulting `BuildOptions` to its
/// `BuildConfigurationKey` (Java `TransitionApplier` followed by
/// `BuildConfigurationKeyMapProducer`).
// Java: com/google/devtools/build/lib/analysis/producers/TransitionApplier.java:76 TransitionApplier.step
pub fn apply_transition(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    label: &Label,
    from_configuration: &BuildConfigurationKey,
    transition: &ConfigurationTransition,
    event_handler: &StoredEventHandler,
) -> Produced<Result<IndexMap<String, BuildConfigurationKey>, TransitionApplierError>> {
    let transition_cache = StarlarkTransitionCache::get();
    let does_starlark_transition = does_starlark_transition(transition);
    if !does_starlark_transition {
        let options = transition.apply(
            &restrict(transition, from_configuration.get_options()),
            event_handler,
            &services.starlark_semantics,
        );
        return build_configuration_key_map(env, options, label);
    }

    let starlark_build_settings = transition_cache.get_all_starlark_build_settings(
        transition,
        &command_line_flag_aliases(from_configuration.get_options()),
    );
    let build_settings_details_value: Arc<StarlarkBuildSettingsDetailsValue> =
        if starlark_build_settings.is_empty() {
            // Quick escape if transition doesn't use any Starlark build settings.
            Arc::new(StarlarkBuildSettingsDetailsValue::empty())
        } else {
            match env.get_value_or_throw::<_, TransitionException>(
                StarlarkBuildSettingsDetailsKey::key(&starlark_build_settings),
            )? {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(None),
                Err(e) => return Ok(Some(Err(TransitionApplierError::Transition(e)))),
            }
        };

    // Java: com/google/devtools/build/lib/analysis/producers/TransitionApplier.java:123 TransitionApplier.applyStarlarkTransition
    let transitioned_options = match transition_cache.compute_if_absent(
        from_configuration.get_options(),
        transition,
        &build_settings_details_value,
        event_handler,
        &services.starlark_semantics,
    ) {
        Ok(o) => o,
        Err(e) => return Ok(Some(Err(TransitionApplierError::Transition(e)))),
    };

    build_configuration_key_map(env, transitioned_options, label)
}

/// Java `BuildConfigurationKeyMapProducer`: the configuration key of every option set, in the
/// order of `options`.
// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyMapProducer.java:72 BuildConfigurationKeyMapProducer.step
pub fn build_configuration_key_map(
    env: &mut Environment<'_>,
    options: IndexMap<String, BuildOptions>,
    _label: &Label,
) -> Produced<Result<IndexMap<String, BuildConfigurationKey>, TransitionApplierError>> {
    // Java enqueues one BuildConfigurationKeyProducer per entry; they run in parallel.
    // PORT: BuildConfigurationKeyProducer's work (platform flags, platform mappings, scopes) is
    // what BUILD_CONFIGURATION_KEY computes (BuildConfigurationKeyFunction delegates to the same
    // producer), so each option set is mapped through that node.
    let keys: Vec<(String, BuildConfigurationKeyValueKey)> = options
        .into_iter()
        .map(|(context, build_options)| {
            (
                context,
                BuildConfigurationKeyValueKey::create(Arc::new(build_options)),
            )
        })
        .collect();
    let sky_keys: Vec<SkyKey> = keys.iter().map(|(_, k)| SkyKey::new(k.clone())).collect();
    let lookup = env.get_values_and_exceptions(&sky_keys)?;

    // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyMapProducer.java:87 BuildConfigurationKeyMapProducer.combineResults
    let mut output: IndexMap<String, BuildConfigurationKey> = IndexMap::new();
    let mut first_error: Option<TransitionApplierError> = None;
    let mut missing = false;
    for (transition_key, key) in &keys {
        match lookup.get_or_throw_with(key, |e| e.downcast_ref::<ConfigurationError>().cloned()) {
            Ok(Some(result_key)) => {
                output.insert(transition_key.clone(), (*result_key).clone());
            }
            Ok(None) => missing = true,
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(match e {
                        ConfigurationError::OptionsParsing(o) => {
                            TransitionApplierError::OptionsParsing(o.get_message().to_owned())
                        }
                        ConfigurationError::PlatformMapping(m) => {
                            TransitionApplierError::PlatformMapping(m)
                        }
                        // PORT: InvalidPlatformException from the target platform lookup.
                        other => TransitionApplierError::PlatformFlags(other.to_string()),
                    });
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Ok(Some(Err(e)));
    }
    if missing {
        return Ok(None);
    }
    Ok(Some(Ok(output)))
}
