//! Port of `analysis/producers/ConfiguredAspectProducer.java`: computes `ConfiguredAspect`s and
//! merges them into a prerequisite.
//!
//! PORT: `PrerequisitesProducer.maybeFilterAspects` enqueues one producer per configuration of
//! the prerequisite and they run concurrently: the aspect keys of all of them are looked up in
//! one Skyframe request. [`produce_configured_aspects`] takes the producers of one
//! `PrerequisitesProducer` together, requests their keys in that order (producer, then
//! `getUsedAspects` order) and then runs each producer's `processResult`.

use std::collections::HashMap;

use indexmap::IndexMap;

use blaze_analysis::aspect_collection::{AspectCollection, build_aspect_key};
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configuredtargets::{MergedConfiguredTarget, MergingException};
use blaze_packages::aspect::AspectDescriptor;
use blaze_skyframe::{Environment, Interrupted, SkyError, SkyKey};
use std::sync::Arc;

use crate::aspect_function::AspectFunctionException;
use crate::producers::errors::{AspectCreationException, TransitiveDependencyState};

/// The errors `ConfiguredAspectProducer.ResultSink` accepts.
#[derive(Clone, Debug)]
pub enum ConfiguredAspectError {
    /// `acceptConfiguredAspectError(AspectCreationException)`.
    AspectCreation(AspectCreationException),
    /// `acceptConfiguredAspectError(MergingException)`.
    Merging(MergingException),
}

/// The outcome of one producer: `None` when a dependency is missing (Skyframe restart), else
/// the merged prerequisite (`acceptConfiguredAspectMergedTarget`) or the errors accepted, in
/// order.
pub type ConfiguredAspectOutcome =
    Option<Result<ConfiguredTargetAndData, Vec<ConfiguredAspectError>>>;

/// Java `ConfiguredTargetKey.fromConfiguredTarget(configuredTarget)`: unwraps a
/// `MergedConfiguredTarget` first (it is ephemeral and has no Skyframe entry).
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:236 ConfiguredTargetKey.fromConfiguredTarget
pub fn configured_target_key_from_configured_target(
    configured_target: &ConfiguredTarget,
) -> ConfiguredTargetKey {
    configured_target
        .unwrap_if_merged()
        .get_lookup_key()
        .as_any()
        .downcast_ref::<ConfiguredTargetKey>()
        .expect("ClassCastException: not a ConfiguredTargetKey")
        .clone()
}

/// The catch clause of an `ASPECT` lookup (Java `tasks.lookUp(key, AspectCreationException.class,
/// sink)`).
pub fn catch_aspect_creation_exception(e: &SkyError) -> Option<AspectCreationException> {
    match e.downcast_ref::<AspectFunctionException>()? {
        AspectFunctionException::AspectCreation(e) => Some(e.clone()),
        _ => None,
    }
}

/// The keys one producer looks up (Java `ConfiguredAspectProducer.step`).
// Java: com/google/devtools/build/lib/analysis/producers/ConfiguredAspectProducer.java:74 ConfiguredAspectProducer.step
fn configured_aspect_keys(
    aspects: &AspectCollection,
    prerequisite: &ConfiguredTargetAndData,
) -> Vec<AspectKey> {
    let base_key =
        configured_target_key_from_configured_target(prerequisite.get_configured_target());
    let mut memo_table: IndexMap<AspectDescriptor, AspectKey> = IndexMap::new();
    aspects
        .get_used_aspects()
        .iter()
        .map(|deps| build_aspect_key(deps, &mut memo_table, &base_key))
        .collect()
}

/// Runs the `ConfiguredAspectProducer`s of `prerequisites` (one per configuration), in order.
// Java: com/google/devtools/build/lib/analysis/producers/ConfiguredAspectProducer.java:41 ConfiguredAspectProducer
pub fn produce_configured_aspects(
    env: &mut Environment<'_>,
    aspects: &AspectCollection,
    prerequisites: Vec<ConfiguredTargetAndData>,
    transitive_state: &mut TransitiveDependencyState,
) -> Result<Vec<ConfiguredAspectOutcome>, Interrupted> {
    // PORT: request order. The `step`s of all producers run before any lookup.
    let keys: Vec<Vec<AspectKey>> = prerequisites
        .iter()
        .map(|prerequisite| configured_aspect_keys(aspects, prerequisite))
        .collect();
    let sky_keys: Vec<SkyKey> = keys
        .iter()
        .flatten()
        .map(|k| SkyKey::new(k.clone()))
        .collect();
    let lookup = env.get_values_and_exceptions(&sky_keys)?;

    let mut outcomes = Vec::with_capacity(prerequisites.len());
    for (prerequisite, keys) in prerequisites.into_iter().zip(keys) {
        // Java: com/google/devtools/build/lib/analysis/producers/ConfiguredAspectProducer.java:87 ConfiguredAspectProducer.acceptValueOrException
        let mut aspect_values: HashMap<AspectDescriptor, Arc<AspectValue>> = HashMap::new();
        let mut errors: Vec<ConfiguredAspectError> = Vec::new();
        let mut missing = false;
        for key in &keys {
            match lookup.get_or_throw_with(key, catch_aspect_creation_exception) {
                Ok(Some(value)) => {
                    aspect_values.insert(value.get_aspect().get_descriptor().clone(), value);
                }
                Ok(None) => missing = true,
                Err(error) => errors.push(ConfiguredAspectError::AspectCreation(error)),
            }
        }
        if !errors.is_empty() {
            outcomes.push(Some(Err(errors)));
            continue;
        }
        if missing {
            outcomes.push(None);
            continue;
        }
        outcomes.push(Some(process_result(
            aspects,
            &prerequisite,
            &aspect_values,
            transitive_state,
        )));
    }
    Ok(outcomes)
}

// Java: com/google/devtools/build/lib/analysis/producers/ConfiguredAspectProducer.java:97 ConfiguredAspectProducer.processResult
fn process_result(
    aspects: &AspectCollection,
    prerequisite: &ConfiguredTargetAndData,
    aspect_values: &HashMap<AspectDescriptor, Arc<AspectValue>>,
    transitive_state: &mut TransitiveDependencyState,
) -> Result<ConfiguredTargetAndData, Vec<ConfiguredAspectError>> {
    let used_aspects = aspects.get_used_aspects();
    let mut configured_aspects: Vec<ConfiguredAspect> = Vec::with_capacity(used_aspects.len());
    for dep_aspect in used_aspects {
        let value = aspect_values
            .get(dep_aspect.aspect())
            .expect("NullPointerException: aspect value");
        // PORT: Java skips `value == ConfiguredAspect.NonApplicableAspect.INSTANCE`, an identity
        // comparison of an `AspectValue` with the `NonApplicableAspect` singleton, which is never
        // true (an `AspectValue` wraps the configured aspect's providers and actions); no value
        // is skipped.
        configured_aspects.push(to_configured_aspect(value));
        if transitive_state.store_transitive_packages() {
            transitive_state.update_transitive_packages_of_aspect(
                value.get_key_for_transitive_package_tracking(),
                value
                    .get_transitive_packages()
                    .expect("NullPointerException: transitive packages"),
            );
        }
    }
    match MergedConfiguredTarget::of(
        prerequisite.get_configured_target().clone(),
        &configured_aspects,
    ) {
        // Java: ConfiguredTargetAndData.fromConfiguredTarget(maybeNew).
        Ok(merged) => Ok(ConfiguredTargetAndData::new(
            merged,
            prerequisite.get_target().clone(),
            prerequisite.get_configuration().cloned(),
            prerequisite.get_transition_keys().to_vec(),
        )),
        Err(e) => Err(vec![ConfiguredAspectError::Merging(e)]),
    }
}

/// An `AspectValue` as the `ConfiguredAspect` it is in Java (`AspectValue implements
/// ConfiguredAspect`).
pub fn to_configured_aspect(value: &Arc<AspectValue>) -> ConfiguredAspect {
    value.get_configured_object()
}
