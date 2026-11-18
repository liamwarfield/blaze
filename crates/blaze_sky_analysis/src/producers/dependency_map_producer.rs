//! Ports of `analysis/producers/DependencyMapProducer.java`,
//! `analysis/producers/DependencyProducer.java`, `analysis/producers/PrerequisitesProducer.java`,
//! `analysis/producers/PrerequisiteParameters.java` and
//! `analysis/producers/AttributeConfiguration.java`.
//!
//! PORT: the state machines become straight-line functions over the blocking evaluator. Java
//! runs the `DependencyProducer`s of one map concurrently and stores each result at its index;
//! here they run one after another in index order, which yields the same map. Materializers
//! (dormant dependencies) and aspects are not ported.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::config::ordered_set_multimap::{
    DependencyLabels, PrerequisiteMap, labels_size,
};
use blaze_analysis::config::starlark_transition::TransitionException;
use blaze_analysis::config::transitions::{
    AttributeTransitionData, PATCH_TRANSITION_KEY, StarlarkAttributeTransitionProvider,
    create_attribute_transition,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_analysis::toolchains::toolchain_context::ToolchainContext;
use blaze_analysis::toolchains::{ToolchainCollection, UnloadedToolchainContext};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler, StoredEventHandler};
use blaze_packages::exceptions::{NoSuchTargetException, NoSuchThingException};
use blaze_packages::target::Target;
use blaze_packages::target::target_utils::{
    format_missing_edge_with_attribute, get_location_maybe,
};
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::Environment;
use indexmap::IndexMap;

use crate::producers::configured_target_and_data_producer::{
    ConfiguredTargetAndDataRequest, produce_configured_targets_and_data,
};
use crate::producers::dependency_resolution_helpers::{
    Aspect, ExecutionPlatformResult, get_execution_platform_label,
};
use crate::producers::errors::{
    Cause, ConfiguredTargetLookupError, ConfiguredValueCreationException, DependencyError,
    DependencyEvaluationException, MissingEdgeError, TargetAndConfiguration,
    TransitiveDependencyState, configuration_id_of_key, event_location,
};
use crate::producers::transition_applier::{Produced, TransitionApplierError, apply_transition};

// -------------------------------------------------------------------------------------------
// PrerequisiteParameters.

/// Common parameters for computing prerequisites (Java `PrerequisiteParameters`).
// Java: com/google/devtools/build/lib/analysis/producers/PrerequisiteParameters.java:38 PrerequisiteParameters
pub struct PrerequisiteParameters<'a> {
    pub services: &'a AnalysisServices,
    pub configured_target_key: &'a ConfiguredTargetKey,
    /// PORT: Java's `MissingEdgeError`s go through the result sink to
    /// `DependencyResolver.State`, which emits them with its `TargetAndConfiguration`; the node
    /// is carried here so they are emitted where they are accepted.
    pub target_and_configuration: &'a TargetAndConfiguration,
    pub aspects: &'a [Aspect],
    /// The Starlark exec transition provider (`None` if Bazel uses native logic).
    pub starlark_transition_provider: Option<Arc<StarlarkAttributeTransitionProvider>>,
    pub toolchain_contexts: Option<&'a ToolchainCollection<UnloadedToolchainContext>>,
    /// Non-null for rules (and output files when there are aspects that apply to files).
    pub attribute_map: Option<&'a ConfiguredAttributeMapper>,
    pub transitive_state: &'a mut TransitiveDependencyState,
    pub event_handler: &'a StoredEventHandler,
}

impl PrerequisiteParameters<'_> {
    pub fn label(&self) -> &Label {
        self.configured_target_key.get_label()
    }

    pub fn target(&self) -> &Target {
        self.target_and_configuration.get_target()
    }

    pub fn configuration_key(&self) -> Option<&BuildConfigurationKey> {
        self.configured_target_key.get_configuration_key()
    }

    pub fn location(&self) -> blaze_starlark::Location {
        self.target().get_location()
    }

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisiteParameters.java:151 PrerequisiteParameters.eventId
    pub fn event_id(&self) -> Option<String> {
        configuration_id_of_key(self.configuration_key())
    }

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisiteParameters.java:156 PrerequisiteParameters.getExecutionPlatformLabel
    pub fn get_execution_platform_label(
        &self,
        exec_group: &str,
        is_base_target_toolchain: bool,
    ) -> Option<Label> {
        if is_base_target_toolchain {
            blaze_util::unported!(
                "PrerequisiteParameters.getExecutionPlatformLabel",
                "aspects"
            );
        }
        let context = self
            .toolchain_contexts
            .expect("NullPointerException: toolchain contexts");
        let platform = context
            .get_toolchain_context(exec_group)
            .expect("NullPointerException: exec group toolchain context")
            .execution_platform();
        Some(platform.label().clone())
    }

    /// Java `DependencyMapProducer.acceptDependencyError(MissingEdgeError)` →
    /// `DependencyResolver.State.acceptDependencyMapError(MissingEdgeError)`.
    fn accept_missing_edge(&mut self, error: MissingEdgeError) {
        error.emit_causes_and_events(
            self.target_and_configuration,
            self.transitive_state,
            self.event_handler,
        );
    }
}

// -------------------------------------------------------------------------------------------
// AttributeConfiguration.

/// The configuration(s) of a dependency edge (Java `AttributeConfiguration`).
// Java: com/google/devtools/build/lib/analysis/producers/AttributeConfiguration.java:24 AttributeConfiguration
#[derive(Clone, Debug)]
enum AttributeConfiguration {
    /// This is a visibility dependency: null configuration, must be a `PackageGroup`.
    Visibility,
    /// The configuration is null (the dependency is in the parent's package and not
    /// configurable); stores any transition keys.
    NullTransitionKeys(Vec<String>),
    /// There is a single configuration (a patch transition or no transition at all).
    Unary(BuildConfigurationKey),
    /// There is a split transition (possibly with only one entry).
    Split(IndexMap<String, BuildConfigurationKey>),
}

// -------------------------------------------------------------------------------------------
// DependencyMapProducer.

/// The outcome of [`produce_dependency_map`] (Java `DependencyMapProducer.ResultSink`).
pub struct DependencyMapOutcome {
    /// `acceptDependencyMap`: `None` when there was an error or a dependency is missing.
    pub dependency_map: Option<PrerequisiteMap>,
    /// `acceptMaterializerTargets` (always `None`: materializers are not ported).
    pub materializer_targets: Option<PrerequisiteMap>,
    /// `acceptDependencyMapError(DependencyError)`: the most important error.
    pub dependency_map_error: Option<DependencyError>,
}

/// Errors seen while computing one map (Java `DependencyMapProducer.lastError`).
struct ErrorState {
    last_error: Option<DependencyError>,
}

impl ErrorState {
    // Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:461 DependencyMapProducer.emitErrorIfMostImportant
    fn accept_dependency_error(&mut self, error: DependencyError) {
        match &self.last_error {
            Some(last) if !DependencyError::is_second_error_more_important(last, &error) => {}
            _ => self.last_error = Some(error),
        }
    }
}

fn is_for_dependency_resolution(dependency_kind: &DependencyKind) -> bool {
    match dependency_kind.get_attribute() {
        None => false,
        Some(a) => a.is_for_dependency_resolution(),
    }
}

/// Computes the configured target dependency map (Java `DependencyMapProducer`).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:339 DependencyMapProducer.step
pub fn produce_dependency_map(
    env: &mut Environment<'_>,
    parameters: &mut PrerequisiteParameters<'_>,
    dependency_labels: &DependencyLabels,
) -> Produced<DependencyMapOutcome> {
    if !parameters.aspects.is_empty() {
        // Java: DependencyMapProducer.computeAspectPropagationEdges
        blaze_util::unported!("DependencyMapProducer.computeAspectPropagationEdges");
    }
    let mut results: Vec<Option<Vec<ConfiguredTargetAndData>>> =
        vec![None; labels_size(dependency_labels)];
    let mut errors = ErrorState { last_error: None };
    let mut missing = false;

    // Java: attributeResolutionStep(tasks, forMaterializers=true, ...), then
    // attributeResolutionStep(tasks, forMaterializers=false, ...).
    for for_materializers in [true, false] {
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:285 DependencyMapProducer.attributeResolutionStep
        let mut index = 0;
        for (kind, labels) in dependency_labels {
            let for_dependency_resolution = is_for_dependency_resolution(kind);
            let skip = for_materializers != for_dependency_resolution;

            // Only call materializer when materialization results are ready
            if !for_materializers {
                if let Some(attribute) = kind.get_attribute() {
                    if attribute.is_materializing() {
                        blaze_util::unported!(
                            "DependencyMapProducer.getMaterializationResultMaybe",
                            "attribute '{}'",
                            attribute.get_name()
                        );
                    }
                }
            }

            // The list of aspects is evaluated here to be done once per attribute, rather than
            // once per dependency.
            // Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:64 AspectResolutionHelpers.computePropagatingAspects
            if !skip {
                if let Some(attribute) = kind.get_attribute() {
                    if attribute.has_aspects() {
                        blaze_util::unported!(
                            "AspectResolutionHelpers.computePropagatingAspects",
                            "aspects on attribute '{}' of {}",
                            attribute.get_name(),
                            parameters.label()
                        );
                    }
                }
            }
            let aspects: Vec<Aspect> = Vec::new();
            for label in labels {
                let current_index = index;
                index += 1;
                if skip {
                    continue;
                }
                let Some(label) = label else {
                    continue;
                };
                match produce_dependency(env, parameters, kind, label, &aspects, &mut errors)? {
                    None => missing = true,
                    Some(None) => {}
                    Some(Some(values)) => results[current_index] = Some(values),
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:428 DependencyMapProducer.buildAndEmitResult
    let dependency_map_error = errors.last_error;
    if dependency_map_error.is_some() || parameters.transitive_state.has_root_cause() {
        return Ok(Some(DependencyMapOutcome {
            dependency_map: None,
            materializer_targets: None,
            dependency_map_error,
        }));
    }
    if missing {
        return Ok(None);
    }

    let mut output: PrerequisiteMap = IndexMap::new();
    let mut i = 0;
    // It's correct to iterate once per entry (not once per key), which is what's needed to keep
    // in sync with `results`.
    for (kind, labels) in dependency_labels {
        for _ in labels {
            let Some(result) = results[i].take() else {
                // There was an error.
                return Ok(Some(DependencyMapOutcome {
                    dependency_map: None,
                    materializer_targets: None,
                    dependency_map_error: None,
                }));
            };
            i += 1;
            // An empty `result` means the entry is skipped due to a missing exec group.
            if !result.is_empty() {
                output.entry(kind.clone()).or_default().extend(result);
            }
        }
    }

    Ok(Some(DependencyMapOutcome {
        dependency_map: Some(output),
        materializer_targets: None,
        dependency_map_error: None,
    }))
}

// -------------------------------------------------------------------------------------------
// DependencyProducer.

/// Computes the values of one dependency edge (Java `DependencyProducer`). `Ok(Some(None))`:
/// an error was accepted; `Ok(Some(Some(values)))`: one value per configuration (empty for a
/// skipped dependency).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:132 DependencyProducer.step
fn produce_dependency(
    env: &mut Environment<'_>,
    parameters: &mut PrerequisiteParameters<'_>,
    kind: &DependencyKind,
    to_label: &Label,
    propagating_aspects: &[Aspect],
    errors: &mut ErrorState,
) -> Produced<Option<Vec<ConfiguredTargetAndData>>> {
    let attribute = kind.get_attribute();

    if *kind == DependencyKind::VISIBILITY_DEPENDENCY
        || attribute.is_some_and(|a| a.get_name() == "visibility")
    {
        // This is always a null transition because visibility targets are not configurable.
        return compute_prerequisites(
            env,
            parameters,
            kind,
            to_label,
            AttributeConfiguration::Visibility,
            None,
            propagating_aspects,
            errors,
        );
    }
    if *kind == DependencyKind::TRANSITIVE_VISIBILITY_DEPENDENCY {
        return compute_prerequisites(
            env,
            parameters,
            kind,
            to_label,
            AttributeConfiguration::Visibility,
            None,
            propagating_aspects,
            errors,
        );
    }

    // The logic of `DependencyResolutionHelpers.computeDependencyLabels` implies that
    // `parameters.configurationKey()` is non-null for everything that follows.
    let configuration_key = parameters
        .configuration_key()
        .expect("NullPointerException: configuration key")
        .clone();

    if kind.is_toolchain() {
        // There's no attribute so no attribute transition.

        // This dependency is a toolchain. Its package has not been loaded and therefore we can't
        // determine which aspects and which rule configuration transition we should use, so just
        // use sensible defaults. Not depending on their package makes the error message
        // reporting a missing toolchain a bit better.
        let execution_platform_label = parameters.get_execution_platform_label(
            kind.get_exec_group_name().expect("toolchain kind"),
            kind.is_base_target_toolchain(),
        );
        return compute_prerequisites(
            env,
            parameters,
            kind,
            to_label,
            AttributeConfiguration::Unary(configuration_key),
            execution_platform_label,
            propagating_aspects,
            errors,
        );
    }

    if *kind == DependencyKind::OUTPUT_FILE_RULE_DEPENDENCY {
        // There's no attribute so no attribute transition.
        return compute_prerequisites(
            env,
            parameters,
            kind,
            to_label,
            AttributeConfiguration::Unary(configuration_key),
            None,
            propagating_aspects,
            errors,
        );
    }

    let attribute = attribute.expect("attribute dependency kind");
    let mut transition_data = AttributeTransitionData {
        attributes: parameters.attribute_map,
        execution_platform: None,
        analysis_data: parameters.starlark_transition_provider.clone(),
    };
    match get_execution_platform_label(
        kind,
        parameters.toolchain_contexts,
        /* base_target_unloaded_toolchain_contexts= */ None,
        parameters.aspects,
    ) {
        ExecutionPlatformResult::Label(l) => transition_data.execution_platform = Some(l),
        ExecutionPlatformResult::NullLabel => transition_data.execution_platform = None,
        ExecutionPlatformResult::Skip => return Ok(Some(Some(Vec::new()))),
        ExecutionPlatformResult::Error(message) => {
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:617 ExecGroupErrorEmitter
            // The configuration value should already exist as a dependency so this lookup is
            // safe enough for error handling.
            if env.get_value(configuration_key)?.is_none() {
                return Ok(None);
            }
            // PORT: AnalysisRootCauseEvent (BEP) is not posted.
            errors.accept_dependency_error(DependencyError::AspectEvaluation(
                DependencyEvaluationException::of_creation(
                    ConfiguredValueCreationException::new(
                        Some(parameters.location()),
                        message,
                        parameters.label(),
                        parameters.event_id(),
                        /* root_causes= */ None,
                        /* detailed_exit_code= */ None,
                    ),
                    // This error originates in dependency resolution, attached to the current
                    // target, so no dependency has reported the error.
                    /* dep_reported_own_error= */
                    false,
                ),
            ));
            return Ok(Some(None));
        }
    }
    let attribute_transition =
        match create_attribute_transition(attribute.get_transition_factory(), &transition_data) {
            Ok(t) => t,
            Err(e) => {
                errors.accept_dependency_error(DependencyError::TransitionCreation(e));
                return Ok(Some(None));
            }
        };
    // PORT: `sink.acceptTransition` feeds cquery's TransitionCollector (not ported).
    let transitioned_configurations = match apply_transition(
        env,
        parameters.services,
        to_label,
        &configuration_key,
        &attribute_transition,
        parameters.event_handler,
    )? {
        None => return Ok(None),
        Some(Ok(t)) => t,
        Some(Err(e)) => {
            let error = match e {
                // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:262 DependencyProducer.acceptTransitionError
                TransitionApplierError::Transition(t) => {
                    DependencyError::DependencyTransition(TransitionException(
                        get_message_with_edge_transition_info(parameters, kind, to_label, &t.0),
                    ))
                }
                // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:268 DependencyProducer.acceptOptionsParsingError
                TransitionApplierError::OptionsParsing(m) => {
                    DependencyError::DependencyOptionsParsing(
                        get_message_with_edge_transition_info(parameters, kind, to_label, &m),
                    )
                }
                TransitionApplierError::BuildOptionsScope(m) => {
                    DependencyError::BuildOptionsScope(m)
                }
                TransitionApplierError::PlatformMapping(m) => DependencyError::PlatformMapping(m),
                TransitionApplierError::PlatformFlags(m) => DependencyError::InvalidPlatform(m),
            };
            errors.accept_dependency_error(error);
            return Ok(Some(None));
        }
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:306 DependencyProducer.processTransitionResult
    let mut is_nonconfigurable_target_in_same_package = false;
    match get_target_in_same_package_without_skyframe(env, parameters.target(), to_label)? {
        SamePackageTarget::Missing => return Ok(None),
        SamePackageTarget::Found(to_target) => {
            is_nonconfigurable_target_in_same_package = !to_target.is_configurable();
        }
        SamePackageTarget::NotInSamePackage => {}
        SamePackageTarget::NoSuchTarget(e) => {
            let parent_target = parameters.target().clone();
            parameters
                .transitive_state
                .add_transitive_cause(Cause::LoadingFailed {
                    label: to_label.clone(),
                    detailed_exit_code: e.get_detailed_exit_code(),
                });
            parameters.event_handler.handle(&Event::error_at(
                event_location(get_location_maybe(&parent_target).as_ref()),
                format_missing_edge_with_attribute(
                    Some(&parent_target),
                    to_label,
                    &NoSuchThingException::Target(e),
                    Some(&**attribute),
                ),
            ));
        }
    }

    if is_nonconfigurable_target_in_same_package {
        // The target is in the same package as the parent and non-configurable. In the general
        // case loading a child target would defeat Package-based sharding. However, when the
        // target is in the same Package, that concern no longer applies.

        // Discards transition keys for patch transitions but keeps them otherwise.
        let transition_keys: Vec<String> = if transitioned_configurations.len() == 1
            && transitioned_configurations.contains_key(PATCH_TRANSITION_KEY)
        {
            Vec::new()
        } else {
            transitioned_configurations.keys().cloned().collect()
        };
        return compute_prerequisites(
            env,
            parameters,
            kind,
            to_label,
            AttributeConfiguration::NullTransitionKeys(transition_keys),
            None,
            propagating_aspects,
            errors,
        );
    }

    // PORT: ConfigurationTransitionEvent (BEP) is not posted.

    if transitioned_configurations.len() == 1 {
        if let Some(patched_configuration) = transitioned_configurations.get(PATCH_TRANSITION_KEY) {
            // It was a patch transition or no-op split transition.
            return compute_prerequisites(
                env,
                parameters,
                kind,
                to_label,
                AttributeConfiguration::Unary(patched_configuration.clone()),
                None,
                propagating_aspects,
                errors,
            );
        }
    }

    compute_prerequisites(
        env,
        parameters,
        kind,
        to_label,
        AttributeConfiguration::Split(transitioned_configurations),
        None,
        propagating_aspects,
        errors,
    )
}

// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:294 DependencyProducer.getMessageWithEdgeTransitionInfo
fn get_message_with_edge_transition_info(
    parameters: &PrerequisiteParameters<'_>,
    kind: &DependencyKind,
    to_label: &Label,
    message: &str,
) -> String {
    format!(
        "on dependency edge {} ({}) -|{}|-> {}: {}",
        parameters.target().get_label(),
        parameters
            .configuration_key()
            .expect("configuration key")
            .get_options()
            .short_id(),
        kind.get_attribute().expect("attribute").get_name(),
        to_label,
        message
    )
}

enum SamePackageTarget {
    NotInSamePackage,
    Found(Target),
    NoSuchTarget(NoSuchTargetException),
    Missing,
}

/// Attempts to resolve a label to a target in the same package as the parent target without
/// a Skyframe call.
///
/// PORT: Blaze's targets do not point back to their package; the parent's package (already
/// computed by the parent's `TargetProducer`) is read from PACKAGE.
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:583 DependencyProducer.getTargetInSamePackageWithoutSkyframe
fn get_target_in_same_package_without_skyframe(
    env: &mut Environment<'_>,
    parent_target: &Target,
    label: &Label,
) -> Result<SamePackageTarget, blaze_skyframe::Interrupted> {
    if parent_target.get_label().get_package_identifier() != label.get_package_identifier() {
        return Ok(SamePackageTarget::NotInSamePackage);
    }
    let Some(parent_pkg) = env.get_value(PackageKey(label.get_package_identifier().clone()))?
    else {
        return Ok(SamePackageTarget::Missing);
    };
    // Throws NoSuchTargetException if label is not found; since parentPkg is a full Package,
    // this guarantees that label is not a valid target.
    Ok(
        match parent_pkg.get_package().get_target(label.get_name()) {
            Ok(t) => SamePackageTarget::Found(t.clone()),
            Err(e) => SamePackageTarget::NoSuchTarget(e),
        },
    )
}

/// `computePrerequisites` followed by `evaluateMaterializerTargets` and `emitResults`.
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:389 DependencyProducer.computePrerequisites
fn compute_prerequisites(
    env: &mut Environment<'_>,
    parameters: &mut PrerequisiteParameters<'_>,
    kind: &DependencyKind,
    to_label: &Label,
    configuration: AttributeConfiguration,
    execution_platform_label: Option<Label>,
    propagating_aspects: &[Aspect],
    errors: &mut ErrorState,
) -> Produced<Option<Vec<ConfiguredTargetAndData>>> {
    let prerequisite_values = match produce_prerequisites(
        env,
        parameters,
        to_label,
        execution_platform_label,
        configuration,
        propagating_aspects,
    )? {
        None => return Ok(None),
        Some(Ok(v)) => v,
        Some(Err(errs)) => {
            for e in errs {
                match e {
                    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:433 DependencyProducer.acceptPrerequisitesError
                    PrerequisitesError::NoSuchThing(error) => parameters.accept_missing_edge(
                        MissingEdgeError::new(kind.clone(), to_label.clone(), error),
                    ),
                    PrerequisitesError::InvalidVisibility(label) => {
                        errors.accept_dependency_error(DependencyError::InvalidVisibility(label))
                    }
                    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:443 DependencyProducer.acceptPrerequisitesCreationError
                    PrerequisitesError::Creation(error) => {
                        // Cases where the child target cannot be loaded at all are propagated as
                        // `NoSuchThingException`. In some cases, child target loading completes
                        // with errors. In that case, the error is propagated as a
                        // `ConfiguredValueCreationException` with a `LoadingFailedCause`.
                        // Requests parent-side context to be added to such errors by
                        // propagating a `MissingEdgeError`.
                        for cause in error.get_root_causes().to_list().iter() {
                            if let Cause::LoadingFailed { label, .. } = cause {
                                if label == to_label {
                                    parameters.accept_missing_edge(MissingEdgeError::new(
                                        kind.clone(),
                                        to_label.clone(),
                                        NoSuchThingException::Target(
                                            NoSuchTargetException::create_for_parent_propagation(
                                                to_label.clone(),
                                            ),
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            return Ok(Some(None));
        }
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:465 DependencyProducer.evaluateMaterializerTargets
    // If the target this DependencyProducer is producing dependencies for is an alias, then do
    // not expand materializer targets.
    if let Some(rule) = parameters.target().get_associated_rule() {
        // Identifying a rule by its ConfiguredTargetFactory is a bit hacky.
        if rule.get_rule_class_object().get_configured_target_factory() == Some("Alias") {
            return Ok(Some(Some(prerequisite_values)));
        }
    }
    for dep in &prerequisite_values {
        // Skip non-materializer rules.
        if let Some(rule) = dep.get_target().as_rule() {
            if rule.get_rule_class_object().is_materializer_rule() {
                blaze_util::unported!(
                    "DependencyProducer.evaluateMaterializerTargets",
                    "materializer target {}",
                    dep.get_target_label()
                );
            }
        }
    }
    Ok(Some(Some(prerequisite_values)))
}

// -------------------------------------------------------------------------------------------
// PrerequisitesProducer.

/// The errors `PrerequisitesProducer.ResultSink` accepts.
enum PrerequisitesError {
    NoSuchThing(NoSuchThingException),
    /// `InvalidVisibilityDependencyException(label)`.
    InvalidVisibility(Label),
    Creation(ConfiguredValueCreationException),
}

/// Computes the configured targets of one dependency edge (Java `PrerequisitesProducer`).
// Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:131 PrerequisitesProducer.step
fn produce_prerequisites(
    env: &mut Environment<'_>,
    parameters: &mut PrerequisiteParameters<'_>,
    label: &Label,
    execution_platform_label: Option<Label>,
    configuration: AttributeConfiguration,
    propagating_aspects: &[Aspect],
) -> Produced<Result<Vec<ConfiguredTargetAndData>, Vec<PrerequisitesError>>> {
    let get_prerequisite_key = |configuration_key: Option<BuildConfigurationKey>| {
        ConfiguredTargetKey::builder()
            .set_label(label.clone())
            .set_configuration_key(configuration_key)
            .set_execution_platform_label(execution_platform_label.clone())
            .build()
    };
    let requests: Vec<ConfiguredTargetAndDataRequest> = match &configuration {
        AttributeConfiguration::Visibility => vec![ConfiguredTargetAndDataRequest {
            key: get_prerequisite_key(None),
            transition_keys: Vec::new(),
        }],
        AttributeConfiguration::NullTransitionKeys(keys) => vec![ConfiguredTargetAndDataRequest {
            key: get_prerequisite_key(None),
            transition_keys: keys.clone(),
        }],
        AttributeConfiguration::Unary(key) => vec![ConfiguredTargetAndDataRequest {
            key: get_prerequisite_key(Some(key.clone())),
            transition_keys: Vec::new(),
        }],
        AttributeConfiguration::Split(split) => split
            .iter()
            .map(|(transition_key, key)| ConfiguredTargetAndDataRequest {
                key: get_prerequisite_key(Some(key.clone())),
                transition_keys: vec![transition_key.clone()],
            })
            .collect(),
    };
    let outcomes =
        produce_configured_targets_and_data(env, &requests, parameters.transitive_state)?;

    let mut configured_targets: Vec<ConfiguredTargetAndData> = Vec::new();
    let mut errors: Vec<PrerequisitesError> = Vec::new();
    let mut missing = false;
    for outcome in outcomes {
        match outcome {
            None => missing = true,
            Some(Ok(v)) => configured_targets.push(v),
            Some(Err(ConfiguredTargetLookupError::NoSuchThing(e))) => {
                errors.push(PrerequisitesError::NoSuchThing(e))
            }
            // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:194 PrerequisitesProducer.acceptConfiguredTargetAndDataError
            Some(Err(ConfiguredTargetLookupError::InconsistentNullConfig(e))) => {
                if matches!(configuration, AttributeConfiguration::Visibility) {
                    // The target was configurable, but used as a visibility dependency. This is
                    // invalid because only `PackageGroup`s are accepted as visibility
                    // dependencies and those are not configurable.
                    errors.push(PrerequisitesError::InvalidVisibility(label.clone()));
                } else {
                    // `configuration.kind()` was `NULL_TRANSITION_KEYS`. This is only used when
                    // the target is in the same package as the parent and not configurable so
                    // this should never happen.
                    panic!("IllegalStateException: {e}");
                }
            }
            Some(Err(ConfiguredTargetLookupError::Creation(e))) => {
                errors.push(PrerequisitesError::Creation(e))
            }
        }
    }
    if !errors.is_empty() {
        return Ok(Some(Err(errors)));
    }
    if missing {
        return Ok(None);
    }

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:213 PrerequisitesProducer.computeConfiguredAspects
    if matches!(configuration, AttributeConfiguration::Visibility) {
        // Verifies that the dependency is a `package_group`. The value is always at index 0
        // because the `VISIBILITY` configuration is always unary.
        if !matches!(
            configured_targets[0].get_configured_target(),
            ConfiguredTarget::PackageGroup(_)
        ) {
            return Ok(Some(Err(vec![PrerequisitesError::InvalidVisibility(
                label.clone(),
            )])));
        }
    }

    let configured_targets = cleanup_values(configured_targets, parameters.label(), label);

    // PORT: `--exec_aspects` (LoadAspectsKey) and aspect filtering are not ported; with no
    // propagating aspects the aspect collection is empty and the values are accepted as is.
    if !propagating_aspects.is_empty() {
        blaze_util::unported!("PrerequisitesProducer.maybeFilterAspects");
    }
    Ok(Some(Ok(configured_targets)))
}

/// Deduplicates split results and orders them (Java `PrerequisitesProducer.cleanupValues`).
// Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:358 PrerequisitesProducer.cleanupValues
fn cleanup_values(
    configured_targets: Vec<ConfiguredTargetAndData>,
    parent_label: &Label,
    label: &Label,
) -> Vec<ConfiguredTargetAndData> {
    if configured_targets.len() == 1 {
        return configured_targets;
    }
    // Otherwise, there was a split transition.

    if configured_targets[0].get_configuration().is_none() {
        // The resulting configurations are null. Aggregates the transition keys.
        let mut keys: Vec<String> = configured_targets[0].get_transition_keys().to_vec();
        for ct in &configured_targets[1..] {
            blaze_util::check_state!(
                ct.get_configuration().is_none(),
                "inconsistent split transition result from {} to {}",
                parent_label,
                label
            );
            keys.extend(ct.get_transition_keys().iter().cloned());
        }
        let first = &configured_targets[0];
        return vec![ConfiguredTargetAndData::new(
            first.get_configured_target().clone(),
            first.get_target().clone(),
            None,
            keys,
        )];
    }

    // Deduplicates entries that have identical configurations and thus identical values,
    // keeping only the first entry with the configuration.
    let mut configured_targets: Vec<Option<ConfiguredTargetAndData>> =
        configured_targets.into_iter().map(Some).collect();
    let mut seen_configurations: HashSet<Option<BuildConfigurationKey>> = HashSet::new();
    let mut first_index = 0;
    for i in 0..configured_targets.len() {
        let key = configured_targets[i]
            .as_ref()
            .unwrap()
            .get_configured_target()
            .get_configuration_key();
        if !seen_configurations.insert(key) {
            // The target at `i` was a duplicate of a previous target. Deletes it by:
            // 1. overwriting it with the first target; and
            // 2. removing the slot previously associated with the first target.
            configured_targets[i] = configured_targets[first_index].take();
            first_index += 1;
        }
    }
    let mut result: Vec<ConfiguredTargetAndData> = configured_targets
        .into_iter()
        .skip(first_index)
        .map(|c| c.expect("slot after first_index"))
        .collect();
    // Java: ConfiguredTargetAndData.SPLIT_DEP_ORDERING
    result.sort_by(|o1, o2| {
        let first = o1.get_configuration().expect("split configuration");
        let second = o2.get_configuration().expect("split configuration");
        first
            .get_mnemonic()
            .cmp(second.get_mnemonic())
            .then_with(|| first.checksum().cmp(second.checksum()))
    });
    result
}
