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
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_build_config::build_configuration_key::{
    BuildConfigurationKey, BuildConfigurationKeyValueKey,
};
use blaze_build_config::build_options::{BuildOptions, ScopeType};
use blaze_build_config::skyframe::ConfigurationError;
use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
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

/// What `build_configuration_key_map` catches from a BUILD_CONFIGURATION_KEY dependency.
enum CaughtBuildConfigurationKeyError {
    Configuration(ConfigurationError),
    /// An `InvalidPlatformException`: its message.
    InvalidPlatform(String),
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
    Ok(apply_transition_with_lookup_rounds(
        env,
        services,
        label,
        from_configuration,
        transition,
        event_handler,
    )?
    .map(|result| result.map(|(configurations, _lookup_rounds)| configurations)))
}

/// [`apply_transition`], and the number of rounds of Skyframe lookups Java's state machines
/// (`TransitionApplier`, `BuildConfigurationKeyMapProducer`, one `BuildConfigurationKeyProducer`
/// per option set and its `PlatformProducer`) make for this transition before they hand the
/// result to `runAfter`, when every looked-up value is available.
///
/// PORT: request order. The `Driver` sends every lookup of one round in one Skyframe request,
/// and state machines that run concurrently (the `DependencyProducer`s of a target) advance
/// round by round; the round in which a producer's transition is done decides where its
/// configured target lookups are in the target's dependency order. The blocking port computes
/// each configuration key through one BUILD_CONFIGURATION_KEY node, so the count is derived
/// here from the conditions under which Java's machines look something up.
// Java: com/google/devtools/build/lib/analysis/producers/TransitionApplier.java:76 TransitionApplier.step
pub fn apply_transition_with_lookup_rounds(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    label: &Label,
    from_configuration: &BuildConfigurationKey,
    transition: &ConfigurationTransition,
    event_handler: &StoredEventHandler,
) -> Produced<Result<(IndexMap<String, BuildConfigurationKey>, usize), TransitionApplierError>> {
    let transition_cache = StarlarkTransitionCache::get();
    let does_starlark_transition = does_starlark_transition(transition);
    if !does_starlark_transition {
        let options = transition.apply(
            &restrict(transition, from_configuration.get_options()),
            event_handler,
            &services.starlark_semantics,
        );
        let lookup_rounds = build_configuration_key_map_lookup_rounds(&options);
        return Ok(build_configuration_key_map(env, options, label)?
            .map(|result| result.map(|configurations| (configurations, lookup_rounds))));
    }

    let starlark_build_settings = transition_cache.get_all_starlark_build_settings(
        transition,
        &command_line_flag_aliases(from_configuration.get_options()),
    );
    // `tasks.lookUp(StarlarkBuildSettingsDetailsValue.key(starlarkBuildSettings), ...)`.
    let starlark_build_settings_lookup_rounds = if starlark_build_settings.is_empty() {
        0
    } else {
        1
    };
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

    let lookup_rounds = starlark_build_settings_lookup_rounds
        + build_configuration_key_map_lookup_rounds(&transitioned_options);
    Ok(
        build_configuration_key_map(env, transitioned_options, label)?
            .map(|result| result.map(|configurations| (configurations, lookup_rounds))),
    )
}

/// The lookup rounds of a `BuildConfigurationKeyMapProducer`: its `BuildConfigurationKeyProducer`s
/// run concurrently, so the map is done with the slowest of them.
// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyMapProducer.java:72 BuildConfigurationKeyMapProducer.step
fn build_configuration_key_map_lookup_rounds(options: &IndexMap<String, BuildOptions>) -> usize {
    options
        .values()
        .map(build_configuration_key_lookup_rounds)
        .max()
        .unwrap_or(0)
}

/// The lookup rounds of one `BuildConfigurationKeyProducer` (see
/// [`apply_transition_with_lookup_rounds`]).
// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:122 BuildConfigurationKeyProducer.step
fn build_configuration_key_lookup_rounds(options: &BuildOptions) -> usize {
    let mut lookup_rounds = 0;
    // Short-circuit if there are no platform options.
    if let Some(platform_options) = options.get::<PlatformOptions>() {
        if platform_options.platforms.len() == 1 {
            // The PlatformProducer's PLATFORM lookup. `checkTargetPlatformFlags` then goes to
            // `mergeFromPlatformMapping`: a platform with flags (`ParsedFlagsValue.mergeWith`) is
            // not ported, BUILD_CONFIGURATION_KEY fails loudly for it.
            lookup_rounds += 1;
        }
        // `mergeFromPlatformMapping`: the PLATFORM_MAPPING lookup.
        lookup_rounds += 1;
    }
    // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:170 BuildConfigurationKeyProducer.findBuildOptionsScopes
    // (Platform mappings without entries leave the Starlark options and their scope types as
    // they are, so the conditions are those of the options before platform processing.)
    let flags_with_incomplete_scope_info = options.get_starlark_options().keys().any(|key| {
        let scope_type = options.get_scope_type_map().get(key);
        scope_type.is_none() || scope_type == Some(&ScopeType::PROJECT)
    });
    if flags_with_incomplete_scope_info {
        // The BUILD_OPTIONS_SCOPE lookup. (`possiblyApplyScopes`' lookup of the baseline
        // configuration needs a PROJECT scope, which BUILD_OPTIONS_SCOPE does not resolve yet:
        // it fails loudly at `BuildOptionsScopeFunction.findProjectFiles`.)
        lookup_rounds += 1;
    }
    lookup_rounds
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
        // Java: the BuildConfigurationKeyProducer's sink accepts an OptionsParsingException, a
        // PlatformMappingException, an InvalidPlatformException (of its PlatformProducer) or a
        // BuildOptionsScopeFunctionException. BUILD_CONFIGURATION_KEY rethrows the first three
        // (BuildConfigurationKeyFunction.compute); InvalidPlatformException is a
        // ToolchainException of kind InvalidPlatform.
        match lookup.get_or_throw_with(key, |e| {
            if let Some(c) = e.downcast_ref::<ConfigurationError>() {
                return Some(CaughtBuildConfigurationKeyError::Configuration(c.clone()));
            }
            e.downcast_ref::<ToolchainException>()
                .filter(|t| t.kind() == ToolchainExceptionKind::InvalidPlatform)
                .map(|t| {
                    CaughtBuildConfigurationKeyError::InvalidPlatform(t.get_message().to_owned())
                })
        }) {
            Ok(Some(result_key)) => {
                output.insert(transition_key.clone(), (*result_key).clone());
            }
            Ok(None) => missing = true,
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(match e {
                        CaughtBuildConfigurationKeyError::Configuration(
                            ConfigurationError::OptionsParsing(o),
                        ) => TransitionApplierError::OptionsParsing(o.get_message().to_owned()),
                        CaughtBuildConfigurationKeyError::Configuration(
                            ConfigurationError::PlatformMapping(m),
                        ) => TransitionApplierError::PlatformMapping(m),
                        // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:254 BuildConfigurationKeyProducer.acceptPlatformInfoError
                        CaughtBuildConfigurationKeyError::InvalidPlatform(m) => {
                            TransitionApplierError::PlatformFlags(m)
                        }
                        // BuildConfigurationKeyFunction throws nothing else (a
                        // BuildOptionsScopeFunctionException is not caught there: see
                        // `find_build_options_scopes` in blaze_build_config).
                        CaughtBuildConfigurationKeyError::Configuration(other) => panic!(
                            "IllegalStateException: unexpected exception of \
                             BUILD_CONFIGURATION_KEY: {other}"
                        ),
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
