//! Port of `analysis/producers/ConfiguredTargetAndDataProducer.java`.
//!
//! PORT: Java enqueues one producer per key and they run concurrently; the blocking port
//! requests a batch of keys together ([`produce_configured_targets_and_data`]) and returns the
//! per-key outcomes in key order.

use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_packages::package::Package;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{Environment, Interrupted, SkyKey};
use indexmap::IndexMap;

use crate::producers::errors::{
    ConfiguredTargetLookupError, TransitiveDependencyState, catch_configured_target_error,
};
use blaze_analysis::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier;

/// Java `ConfiguredTarget.getLabel()`: for an alias, the label of the target it points to.
// Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:177 AliasConfiguredTarget.getLabel
pub fn configured_target_label(configured_target: &ConfiguredTarget) -> Label {
    match configured_target {
        ConfiguredTarget::Alias(_) => configured_target.get_actual().get_label(),
        other => other.get_label(),
    }
}

/// One request of [`produce_configured_targets_and_data`].
pub struct ConfiguredTargetAndDataRequest {
    pub key: ConfiguredTargetKey,
    pub transition_keys: Vec<String>,
}

/// The outcome of one request: `None` when a dependency is missing.
pub type ConfiguredTargetAndDataOutcome =
    Option<Result<ConfiguredTargetAndData, ConfiguredTargetLookupError>>;

/// Looks up configured targets and wraps each with its `Target` and configuration (Java
/// `ConfiguredTargetAndDataProducer` for each request).
///
/// With `base_target_prerequisites_supplier` (aspect evaluation), the configured target and
/// configuration values are first taken from the graph without a dependency edge, and looked up
/// only when the supplier has none.
// Java: com/google/devtools/build/lib/analysis/producers/ConfiguredTargetAndDataProducer.java:103 ConfiguredTargetAndDataProducer.step
pub fn produce_configured_targets_and_data(
    env: &mut Environment<'_>,
    requests: &[ConfiguredTargetAndDataRequest],
    transitive_state: &mut TransitiveDependencyState,
    base_target_prerequisites_supplier: Option<&BaseTargetPrerequisitesSupplier>,
) -> Result<Vec<ConfiguredTargetAndDataOutcome>, Interrupted> {
    // `baseTargetPrerequisitesSupplier.getPrerequisite(key)`; null → `tasks.lookUp(key, ...)`.
    let cached: Vec<Option<Arc<ConfiguredTargetValue>>> = requests
        .iter()
        .map(|r| base_target_prerequisites_supplier.and_then(|s| s.get_prerequisite(&r.key)))
        .collect();
    let sky_keys: Vec<SkyKey> = requests
        .iter()
        .zip(&cached)
        .filter(|(_, c)| c.is_none())
        .map(|(r, _)| SkyKey::new(r.key.clone()))
        .collect();
    let lookup = env.get_values_and_exceptions(&sky_keys)?;

    // Java: com/google/devtools/build/lib/analysis/producers/ConfiguredTargetAndDataProducer.java:136 ConfiguredTargetAndDataProducer.acceptValueOrException3
    let mut values: Vec<Option<Result<Arc<ConfiguredTargetValue>, ConfiguredTargetLookupError>>> =
        Vec::with_capacity(requests.len());
    for (request, cached) in requests.iter().zip(cached) {
        let value_or_error = match cached {
            Some(v) => Ok(Some(v)),
            None => lookup.get_or_throw_with(&request.key, catch_configured_target_error),
        };
        match value_or_error {
            Ok(Some(value)) => {
                // Java: ConfiguredTargetAndDataProducer.acceptValue
                if transitive_state.store_transitive_packages() {
                    // ConfiguredTargetKey.fromConfiguredTarget(value.getConfiguredTarget())
                    let lookup_key = value.get_configured_target().unwrap_if_merged();
                    let key = lookup_key
                        .get_lookup_key()
                        .as_any()
                        .downcast_ref::<ConfiguredTargetKey>()
                        .expect("ClassCastException: not a ConfiguredTargetKey")
                        .clone();
                    transitive_state.update_transitive_packages_of_configured_target(
                        &key,
                        value
                            .get_transitive_packages()
                            .expect("NullPointerException: transitive packages"),
                    );
                }
                values.push(Some(Ok(value)));
            }
            Ok(None) => values.push(None),
            Err(error) => {
                if let ConfiguredTargetLookupError::Creation(e) = &error {
                    transitive_state.add_transitive_causes(e.get_root_causes());
                }
                values.push(Some(Err(error)));
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/ConfiguredTargetAndDataProducer.java:158 ConfiguredTargetAndDataProducer.fetchConfigurationAndPackage
    let mut configuration_keys: IndexMap<BuildConfigurationKey, ()> = IndexMap::new();
    let mut supplied_configurations: IndexMap<BuildConfigurationKey, Arc<BuildConfigurationValue>> =
        IndexMap::new();
    let mut package_keys: IndexMap<PackageIdentifier, ()> = IndexMap::new();
    for value in values.iter().flatten().flatten() {
        let configured_target = value.get_configured_target();
        if let Some(configuration_key) = configured_target.get_configuration_key() {
            // `baseTargetPrerequisitesSupplier.getPrerequisiteConfiguration(configurationKey)`;
            // null → `tasks.lookUp(configurationKey, this)`.
            match base_target_prerequisites_supplier
                .and_then(|s| s.get_prerequisite_configuration(&configuration_key))
            {
                Some(c) => {
                    supplied_configurations.insert(configuration_key, c);
                }
                None => {
                    configuration_keys.insert(configuration_key, ());
                }
            }
        }
        // An alternative to this is to optimistically fetch the package using the label of the
        // configured target key. However, the actual package may differ when this is an
        // AliasConfiguredTarget and would need to be refetched.
        let package_id = configured_target_label(configured_target)
            .get_package_identifier()
            .clone();
        package_keys.insert(package_id, ());
    }
    let mut dep_keys: Vec<SkyKey> = Vec::new();
    for k in configuration_keys.keys() {
        dep_keys.push(SkyKey::new(k.clone()));
    }
    for p in package_keys.keys() {
        dep_keys.push(SkyKey::new(PackageKey(p.clone())));
    }
    let deps = env.get_values_and_exceptions(&dep_keys)?;
    let configurations: IndexMap<BuildConfigurationKey, Option<Arc<BuildConfigurationValue>>> =
        configuration_keys
            .keys()
            .map(|k| (k.clone(), deps.get_typed(k)))
            .collect();
    let packages: IndexMap<PackageIdentifier, Option<Arc<Package>>> = package_keys
        .keys()
        .map(|p| {
            (
                p.clone(),
                deps.get_typed(&PackageKey(p.clone()))
                    .map(|v| v.get_package().clone()),
            )
        })
        .collect();

    // Java: com/google/devtools/build/lib/analysis/producers/ConfiguredTargetAndDataProducer.java:213 ConfiguredTargetAndDataProducer.constructResult
    let mut outcomes = Vec::with_capacity(requests.len());
    for (request, value) in requests.iter().zip(values) {
        let value = match value {
            None => {
                outcomes.push(None);
                continue;
            }
            Some(Err(e)) => {
                outcomes.push(Some(Err(e)));
                continue;
            }
            Some(Ok(v)) => v,
        };
        let configured_target = value.get_configured_target();
        let configuration_value = match configured_target.get_configuration_key() {
            None => None,
            Some(k) => match supplied_configurations
                .get(&k)
                .cloned()
                .or_else(|| configurations.get(&k).cloned().flatten())
            {
                Some(c) => Some(c),
                None => {
                    outcomes.push(None);
                    continue;
                }
            },
        };
        let label = configured_target_label(configured_target);
        let Some(pkg) = packages
            .get(label.get_package_identifier())
            .cloned()
            .flatten()
        else {
            outcomes.push(None);
            continue;
        };
        let target = match pkg.get_target(label.get_name()) {
            Ok(t) => t.clone(),
            // The package was fetched based on the label of the configured target. Since the
            // configured target exists, it must have existed in the package when it was created.
            Err(e) => panic!(
                "IllegalStateException: Target already verified for {}: {}",
                label, e
            ),
        };
        outcomes.push(Some(Ok(ConfiguredTargetAndData::new(
            configured_target.clone(),
            target,
            configuration_value,
            request.transition_keys.clone(),
        ))));
    }
    Ok(outcomes)
}
