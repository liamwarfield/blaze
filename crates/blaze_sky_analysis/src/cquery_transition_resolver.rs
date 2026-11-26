//! The dependency resolution `query2/cquery/CqueryTransitionResolver.dependencies` runs for
//! cquery's `--output=transitions`.
//!
//! PORT: Java runs `DependencyResolver.evaluate` outside Skyframe, over a `LookupEnvironment` of
//! the graph analysis left (`ConfiguredTargetAccessor.getLookupEnvironment()`), with a state made
//! by `DependencyResolver.State.createForCquery`; a value missing from the graph makes the
//! evaluation incomplete. Blaze's `DependencyResolver` runs on a Skyframe `Environment` only, so
//! this function runs it as a Skyframe node of its own (CQUERY_TRANSITION_RESOLVER, a Blaze-only
//! key) over the same graph: every value it requests for a configured target of the analyzed graph
//! is already done, so the requests read the graph as Java's lookup environment does.

use std::fmt;
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::ordered_set_multimap::PrerequisiteMap;
use blaze_analysis::config::transitions::ConfigurationTransition;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, FunctionHermeticity, Key, SkyFunctionName,
};
use parking_lot::Mutex;

use crate::dependency_resolver::{
    DependencyResolver, DependencyResolverException, DependencyResolverState,
};
use crate::producers::errors::TargetAndConfiguration;

/// The configured target whose dependencies `CqueryTransitionResolver.dependencies` resolves.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CqueryTransitionResolverKey(pub ConfiguredTargetKey);

impl fmt::Debug for CqueryTransitionResolverKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:?}", Self::FUNCTION.0, self.0)
    }
}

impl Key for CqueryTransitionResolverKey {
    type Value = CqueryTransitionResolverValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("CQUERY_TRANSITION_RESOLVER");
    // A function of the analysis graph only.
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
}

/// What `DependencyResolver.evaluate` produced for the target.
#[derive(Debug)]
pub enum CqueryTransitionResolverValue {
    /// `producer.getDepValueMap()` and the transitions the attribute transition collector saw
    /// (`HashBasedTable.put(kind, label, transition)`, in the order they were seen).
    Resolved {
        dep_value_map: PrerequisiteMap,
        transitions: Vec<(DependencyKind, Label, ConfigurationTransition)>,
    },
    /// `IncompatibleTargetChecker.IncompatibleTargetException`.
    IncompatibleTarget,
    /// The message of the `ReportedException` / `UnreportedException`
    /// (`new EvaluateException(e.getMessage())`).
    Error(String),
}

/// Registers CQUERY_TRANSITION_RESOLVER.
pub fn register(builder: EvaluatorBuilder, services: Arc<AnalysisServices>) -> EvaluatorBuilder {
    builder.typed_function::<CqueryTransitionResolverKey, _>(move |key, env| {
        compute(&services, key, env)
    })
}

// Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:130 CqueryTransitionResolver.dependencies
fn compute(
    services: &AnalysisServices,
    key: &CqueryTransitionResolverKey,
    env: &mut Environment<'_>,
) -> Result<Option<CqueryTransitionResolverValue>, ComputeError> {
    let configured_target_key = &key.0;
    // Target target = accessor.getTarget(configuredTarget)
    let label = configured_target_key.get_label();
    let Some(package_value) =
        env.get_value(PackageKey::key(label.get_package_identifier().clone()))?
    else {
        return Ok(None);
    };
    let target = match package_value.get_package().get_target(label.get_name()) {
        Ok(t) => t.clone(),
        Err(e) => panic!("IllegalStateException: {}", e.get_message()),
    };
    // cqueryThreadsafeCallback.getConfiguration(configuredTarget.getConfigurationKey())
    let configuration = match configured_target_key.get_configuration_key() {
        None => None,
        Some(config_key) => {
            let config_key: BuildConfigurationKey = config_key.clone();
            match env.get_value(config_key)? {
                None => return Ok(None),
                Some(c) => Some(c),
            }
        }
    };

    let target_and_configuration = TargetAndConfiguration::new(target, configuration);
    // HashBasedTable.<DependencyKind, Label, ConfigurationTransition>create()
    let collected: Arc<Mutex<Vec<(DependencyKind, Label, ConfigurationTransition)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let sink = collected.clone();
    let mut state = DependencyResolverState::create_for_cquery(
        target_and_configuration.clone(),
        Arc::new(move |kind, label, transition| {
            sink.lock()
                .push((kind.clone(), label.clone(), transition.clone()));
        }),
    );

    let mut producer = DependencyResolver::new(target_and_configuration);
    match producer.evaluate(&mut state, configured_target_key, services, env) {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(DependencyResolverException::Interrupted(e)) => return Err(e.into()),
        Err(DependencyResolverException::IncompatibleTarget(_)) => {
            return Ok(Some(CqueryTransitionResolverValue::IncompatibleTarget));
        }
        Err(DependencyResolverException::Reported(e))
        | Err(DependencyResolverException::Unreported(e)) => {
            return Ok(Some(CqueryTransitionResolverValue::Error(
                e.get_message().to_owned(),
            )));
        }
    }

    if state.transitive_state.has_root_cause() {
        blaze_util::unported!(
            "CqueryTransitionResolver.dependencies",
            "\"expected empty: \" + state.transitiveRootCauses().build().toList() for {:?}",
            configured_target_key
        );
    }

    let transitions = collected.lock().clone();
    Ok(Some(CqueryTransitionResolverValue::Resolved {
        dep_value_map: producer.get_dep_value_map().clone(),
        transitions,
    }))
}
