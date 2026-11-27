//! Ports of `analysis/producers/DependencyMapProducer.java`,
//! `analysis/producers/DependencyProducer.java`, `analysis/producers/PrerequisitesProducer.java`,
//! `analysis/producers/PrerequisiteParameters.java` and
//! `analysis/producers/AttributeConfiguration.java`.
//!
//! PORT: the state machines become straight-line functions over the blocking evaluator. Java
//! runs the `DependencyProducer`s of one map concurrently and stores each result at its index.
//! Here the part of each producer before its `PrerequisitesProducer` runs first, in index order
//! (`plan_dependency`); the configured targets are then requested in the order of Java's
//! lookup rounds (see `produce_dependency_map`, "request order") and each result is stored at
//! its index, which yields the same map. Materializers (dormant dependencies) are not ported.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::aspect_collection::AspectCollection;
use blaze_analysis::aspect_key_creator::LoadAspectsKey;
use blaze_analysis::aspect_resolution_helpers::{
    AspectMultimap, ComputeAspectCollectionError, compute_aspect_collection,
    compute_aspect_collection_no_aspects_filtering, compute_attribute_aspects,
    compute_propagating_aspects, compute_toolchains_aspects,
};
use blaze_analysis::config::configuration_transition_event::ConfigurationTransitionEvent;
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
use blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent;
use blaze_analysis::toolchains::toolchain_context::ToolchainContext;
use blaze_analysis::toolchains::{ToolchainCollection, UnloadedToolchainContext};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler, ExtendedEventHandler, StoredEventHandler};
use blaze_packages::exceptions::{NoSuchTargetException, NoSuchThingException};
use blaze_packages::target::target_utils::{
    format_missing_edge_with_attribute, get_location_maybe,
};
use blaze_packages::target::{Rule, Target};
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::Environment;
use indexmap::IndexMap;

use crate::producers::configured_aspect_producer::{
    ConfiguredAspectError, produce_configured_aspects,
};
use crate::producers::configured_target_and_data_producer::{
    ConfiguredTargetAndDataRequest, produce_configured_targets_and_data,
};
use crate::producers::dependency_resolution_helpers::{
    Aspect, ExecutionPlatformResult, get_execution_platform_label,
};
use crate::producers::errors::{
    AspectCreationException, Cause, ConfiguredTargetLookupError, ConfiguredValueCreationException,
    DependencyError, DependencyEvaluationException, MissingEdgeError, TargetAndConfiguration,
    TransitiveDependencyState, configuration_id_of_key, event_location,
};
use crate::producers::transition_applier::{
    Produced, TransitionApplierError, apply_transition_with_lookup_rounds,
};
use blaze_analysis::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier;

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
    /// This is the key for loading the aspects passed to the --exec_aspects flag, which get
    /// attached to targets in the exec configuration.
    pub load_exec_aspects_key: Option<&'a LoadAspectsKey>,
    /// The Starlark exec transition provider (`None` if Bazel uses native logic).
    pub starlark_transition_provider: Option<Arc<StarlarkAttributeTransitionProvider>>,
    pub toolchain_contexts: Option<&'a ToolchainCollection<UnloadedToolchainContext>>,
    /// Non-null for rules (and output files when there are aspects that apply to files).
    pub attribute_map: Option<&'a ConfiguredAttributeMapper>,
    pub transitive_state: &'a mut TransitiveDependencyState,
    pub event_handler: &'a StoredEventHandler,
    /// Cache for `ConfiguredTargetValue` and `BuildConfigurationValue` (see
    /// `AspectFunction`'s `baseTargetPrerequisitesSupplier`).
    pub base_target_prerequisites_supplier: Option<&'a BaseTargetPrerequisitesSupplier>,
    /// The `UnloadedToolchainContext`s for the base target of the aspect under evaluation.
    ///
    /// This is only non-null during aspect evaluation if the aspects path can propagate to
    /// toolchains.
    pub base_target_toolchain_contexts: Option<&'a ToolchainCollection<UnloadedToolchainContext>>,
    /// Java: the result sink's `acceptTransition` (`DependencyResolver.State` forwards it to its
    /// `TransitionCollector`).
    pub transition_collector: Option<crate::dependency_resolver::TransitionCollector>,
}

impl PrerequisiteParameters<'_> {
    pub fn label(&self) -> &Label {
        self.configured_target_key.get_label()
    }

    pub fn target(&self) -> &Target {
        self.target_and_configuration.get_target()
    }

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisiteParameters.java:118 PrerequisiteParameters.associatedRule
    pub fn associated_rule(&self) -> Option<&Arc<Rule>> {
        self.target().get_associated_rule()
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
        let context = if is_base_target_toolchain {
            self.base_target_toolchain_contexts
        } else {
            self.toolchain_contexts
        };

        // PORT: the Rust ToolchainContext's execution platform is never null.
        let platform = context
            .expect("NullPointerException: toolchain contexts")
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
    let mut errors = ErrorState { last_error: None };
    let (computed_attribute_aspects, computed_toolchains_aspects) =
        match compute_aspect_propagation_edges(parameters, dependency_labels) {
            Ok(computed) => computed,
            Err(e) => {
                parameters.event_handler.handle(&Event::error_at(
                    event_location(Some(&parameters.location())),
                    e.message_with_stack(),
                ));
                errors.accept_dependency_error(DependencyError::AspectEvaluation(
                    DependencyEvaluationException::of_eval(
                        e.message.clone(),
                        Some(parameters.location()),
                    ),
                ));
                return Ok(Some(DependencyMapOutcome {
                    dependency_map: None,
                    materializer_targets: None,
                    dependency_map_error: errors.last_error,
                }));
            }
        };
    let empty_attribute_aspects: AspectMultimap<String> = IndexMap::new();
    let empty_toolchains_aspects: AspectMultimap<Label> = IndexMap::new();
    let mut results: Vec<Option<Vec<ConfiguredTargetAndData>>> =
        vec![None; labels_size(dependency_labels)];
    let mut missing = false;

    // Java: attributeResolutionStep(tasks, forMaterializers=true, ...), then
    // attributeResolutionStep(tasks, forMaterializers=false, ...).
    for for_materializers in [true, false] {
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:285 DependencyMapProducer.attributeResolutionStep
        let mut pending: Vec<PendingPrerequisites<'_>> = Vec::new();
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
            // PORT: Java passes the multimaps (null without aspects) and the possibly null
            // associated rule; W3's port takes references. Without aspects the multimaps are
            // never read, and a target without an associated rule (input file, package group,
            // environment group) has no attribute or base target toolchain dependencies, for
            // which Java returns an empty list.
            let aspects: Vec<Aspect> = if skip {
                Vec::new()
            } else {
                match parameters.associated_rule() {
                    Some(rule) => compute_propagating_aspects(
                        kind,
                        parameters.aspects,
                        computed_attribute_aspects
                            .as_ref()
                            .unwrap_or(&empty_attribute_aspects),
                        computed_toolchains_aspects
                            .as_ref()
                            .unwrap_or(&empty_toolchains_aspects),
                        rule,
                        parameters.base_target_toolchain_contexts,
                    ),
                    None => {
                        blaze_util::check_state!(
                            kind.get_attribute().is_none() && !kind.is_base_target_toolchain(),
                            "{:?} of a target without a rule",
                            kind
                        );
                        Vec::new()
                    }
                }
            };
            for label in labels {
                let current_index = index;
                index += 1;
                if skip {
                    continue;
                }
                let Some(label) = label else {
                    continue;
                };
                // `tasks.enqueue(new DependencyProducer(...))`: the part of the producer that
                // runs before its PrerequisitesProducer.
                match plan_dependency(env, parameters, kind, label, &mut errors)? {
                    PlannedDependency::Missing => missing = true,
                    PlannedDependency::Error => {}
                    PlannedDependency::Skipped => results[current_index] = Some(Vec::new()),
                    PlannedDependency::Prerequisites {
                        configuration,
                        execution_platform_label,
                        lookup_rounds,
                    } => pending.push(PendingPrerequisites {
                        index: current_index,
                        kind,
                        label,
                        configuration,
                        execution_platform_label,
                        lookup_rounds,
                        propagating_aspects: aspects.clone(),
                    }),
                }
            }
        }

        // PORT: request order. Java runs the DependencyProducers of one step concurrently, and
        // the `Driver` puts every `tasks.lookUp` of one round into one Skyframe request (one dep
        // group, in the order the tasks ran). A producer without a TransitionApplier
        // (visibility, toolchain, output file rule) looks its configured target up in the first
        // round; an attribute dependency does so after the lookup rounds of its transition
        // (`lookup_rounds`), a round of 0 after the first group's producers. The configured
        // target keys are requested here in that order: one request per round, in
        // `dependencyLabels` order within the round; `compute_prerequisites` then reads them.
        //
        // PORT: not reproduced, the part of Java's order that depends on which values are done
        // when they are looked up (scripts/w4/scenarios/graph-order, `//pkg:tool`). `Driver.drive`
        // (skyframe/state/Driver.java:83) loops: run every ready task, then look up the keys
        // they added in one request; a lookup whose value is done continues in the next
        // iteration of the same pass, one whose value is missing moves to `pending` and
        // continues only after the Skyframe restart, that is after every round the other
        // producers can reach without a restart. `lookup_rounds` counts iterations as if every
        // looked-up value were done. The lookups before a configured target lookup are
        // STARLARK_BUILD_SETTINGS_DETAILS (TransitionApplier), PLATFORM (PlatformProducer, when
        // `--platforms` has one value), PLATFORM_MAPPING and BUILD_OPTIONS_SCOPE
        // (BuildConfigurationKeyProducer). For the target configuration PLATFORM (the node's own
        // target platform) and PLATFORM_MAPPING are dependencies the node already has, so they
        // are done. The PLATFORM node of the execution platform is not when this node is the
        // first of the build to transition to that platform: in graph-order's DEPS.txt
        // `//pkg:top` requests PLATFORM(@platforms//host:host) in group 9, restarts, and
        // requests `//pkg:tool` and the allowlist (exec transition, 2 rounds) in group 12, after
        // the no-transition dependencies (2 rounds as well) of group 10; a node that finds the
        // PLATFORM value done requests both in the same group, in `dependencyLabels` order, as
        // is done here for every node. The same holds for the first requester of a
        // STARLARK_BUILD_SETTINGS_DETAILS or BUILD_OPTIONS_SCOPE key. Reproducing it needs to
        // know, when a key is requested, whether its value is done (Java:
        // `SkyframeLookupResult.queryDep` returning false); `blaze_skyframe::Environment` has no
        // such query (a request blocks until the value is computed), and the evaluation order of
        // the blocking evaluator differs from Java's, so "first requester" would not be the same
        // node in general.
        pending.sort_by_key(|p| lookup_order_key(p.lookup_rounds, p.index));
        let mut next = 0;
        while next < pending.len() {
            let round = pending[next].lookup_rounds.unwrap_or(0);
            let mut end = next;
            let mut round_keys: Vec<blaze_skyframe::SkyKey> = Vec::new();
            while end < pending.len() && pending[end].lookup_rounds.unwrap_or(0) == round {
                let p = &pending[end];
                // Java: ConfiguredTargetAndDataProducer.step looks a key up only when the
                // `BaseTargetPrerequisitesSupplier` (used during aspect evaluation for the base
                // target's own dependencies) does not have its value: the supplier adds no
                // dependency edge, and neither may this batch.
                // Java: com/google/devtools/build/lib/analysis/producers/ConfiguredTargetAndDataProducer.java:99 ConfiguredTargetAndDataProducer.step
                let supplier = if use_base_target_prerequisites_supplier(parameters, p.kind) {
                    parameters.base_target_prerequisites_supplier
                } else {
                    None
                };
                for request in prerequisite_requests(
                    p.label,
                    p.execution_platform_label.as_ref(),
                    &p.configuration,
                ) {
                    if supplier.is_some_and(|s| s.get_prerequisite(&request.key).is_some()) {
                        continue;
                    }
                    round_keys.push(blaze_skyframe::SkyKey::new(request.key));
                }
                end += 1;
            }
            // The values are read by `compute_prerequisites` below (a missing or failed
            // dependency is handled there, as each ConfiguredTargetAndDataProducer does).
            let _ = env.get_values_and_exceptions(&round_keys)?;
            for p in &pending[next..end] {
                match compute_prerequisites(
                    env,
                    parameters,
                    p.kind,
                    p.label,
                    p.configuration.clone(),
                    p.execution_platform_label.clone(),
                    &p.propagating_aspects,
                    &mut errors,
                )? {
                    None => missing = true,
                    Some(None) => {}
                    Some(Some(values)) => results[p.index] = Some(values),
                }
            }
            next = end;
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

/// Computes the aspects' propagation attribute names and toolchain types (Java
/// `computedAttributeAspects` and `computedToolchainsAspects`; both `None` without aspects).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyMapProducer.java:361 DependencyMapProducer.computeAspectPropagationEdges
fn compute_aspect_propagation_edges(
    parameters: &PrerequisiteParameters<'_>,
    dependency_labels: &DependencyLabels,
) -> Result<
    (
        Option<AspectMultimap<String>>,
        Option<AspectMultimap<Label>>,
    ),
    blaze_starlark::EvalError,
> {
    if parameters.aspects.is_empty() {
        return Ok((None, None));
    }

    // Java passes `parameters.attributeMap()`, null for a target that is not a rule.
    let attribute_map = parameters.attribute_map;
    let computed_attribute_aspects = compute_attribute_aspects(
        parameters.aspects,
        parameters.target(),
        attribute_map,
        dependency_labels,
        parameters.event_handler,
    )?;
    let computed_toolchains_aspects = compute_toolchains_aspects(
        parameters.aspects,
        parameters.target(),
        attribute_map,
        dependency_labels,
        parameters.event_handler,
    )?;
    Ok((
        Some(computed_attribute_aspects),
        Some(computed_toolchains_aspects),
    ))
}

// -------------------------------------------------------------------------------------------
// DependencyProducer.

/// Where the configured target lookups of a dependency are in the order of a target's Skyframe
/// requests: the round after `lookup_rounds` rounds of transition lookups; in the first round,
/// the producers without a TransitionApplier (`None`) before the others; `dependencyLabels`
/// order (`index`) within a round. See the comment in [`produce_dependency_map`].
fn lookup_order_key(lookup_rounds: Option<usize>, index: usize) -> (usize, bool, usize) {
    (lookup_rounds.unwrap_or(0), lookup_rounds.is_some(), index)
}

/// A dependency whose `PrerequisitesProducer` has not run yet.
struct PendingPrerequisites<'a> {
    /// The index of the entry of `dependencyLabels`.
    index: usize,
    kind: &'a DependencyKind,
    label: &'a Label,
    configuration: AttributeConfiguration,
    execution_platform_label: Option<Label>,
    lookup_rounds: Option<usize>,
    propagating_aspects: Vec<Aspect>,
}

/// What a `DependencyProducer` decides before its `PrerequisitesProducer` runs.
enum PlannedDependency {
    /// A dependency is missing (Skyframe restart).
    Missing,
    /// An error was accepted.
    Error,
    /// `sink.acceptDependencyValues(index, EMPTY_OUTPUT)`: the dependency is skipped (see
    /// `DependencyResolutionHelpers.getExecutionPlatformLabel`).
    Skipped,
    /// `computePrerequisites(configuration, executionPlatformLabel)`.
    Prerequisites {
        configuration: AttributeConfiguration,
        execution_platform_label: Option<Label>,
        /// `None`: the producer goes straight to its PrerequisitesProducer. `Some(n)`: it first
        /// runs a TransitionApplier, whose state machines need `n` rounds of Skyframe lookups
        /// (see `apply_transition_with_lookup_rounds`).
        lookup_rounds: Option<usize>,
    },
}

/// The part of `DependencyProducer` before `computePrerequisites` (Java `DependencyProducer.step`,
/// `TransitionApplier` and `processTransitionResult`): the configuration(s) of one dependency
/// edge.
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:132 DependencyProducer.step
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:125 DependencyProducer.<init>
// PORT: skipped, the state machine's result sink is the return value of the synchronous port (the dependency's PrerequisiteMap entry or the DependencyError the caller collects): DependencyProducer.ResultSink.acceptDependencyValues, DependencyProducer.ResultSink.acceptMaterializerTarget, DependencyProducer.ResultSink.acceptDependencyError
// PORT: skipped, materializer rules (dormant dependencies) are not ported (see the module documentation): DependencyProducer.MaterializerRuleDependencySink.acceptTransition, DependencyProducer.MaterializerRuleDependencySink.acceptDependencyValues, DependencyProducer.MaterializerRuleDependencySink.acceptMaterializerTarget, DependencyProducer.MaterializerRuleDependencySink.acceptDependencyError
fn plan_dependency(
    env: &mut Environment<'_>,
    parameters: &mut PrerequisiteParameters<'_>,
    kind: &DependencyKind,
    to_label: &Label,
    errors: &mut ErrorState,
) -> Result<PlannedDependency, blaze_skyframe::Interrupted> {
    let attribute = kind.get_attribute();

    if *kind == DependencyKind::VISIBILITY_DEPENDENCY
        || attribute.is_some_and(|a| a.get_name() == "visibility")
    {
        // This is always a null transition because visibility targets are not configurable.
        return Ok(PlannedDependency::Prerequisites {
            configuration: AttributeConfiguration::Visibility,
            execution_platform_label: None,
            lookup_rounds: None,
        });
    }
    if *kind == DependencyKind::TRANSITIVE_VISIBILITY_DEPENDENCY {
        return Ok(PlannedDependency::Prerequisites {
            configuration: AttributeConfiguration::Visibility,
            execution_platform_label: None,
            lookup_rounds: None,
        });
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
        return Ok(PlannedDependency::Prerequisites {
            configuration: AttributeConfiguration::Unary(configuration_key),
            execution_platform_label,
            lookup_rounds: None,
        });
    }

    if *kind == DependencyKind::OUTPUT_FILE_RULE_DEPENDENCY {
        // There's no attribute so no attribute transition.
        return Ok(PlannedDependency::Prerequisites {
            configuration: AttributeConfiguration::Unary(configuration_key),
            execution_platform_label: None,
            lookup_rounds: None,
        });
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
        parameters.base_target_toolchain_contexts,
        parameters.aspects,
    ) {
        ExecutionPlatformResult::Label(l) => transition_data.execution_platform = Some(l),
        ExecutionPlatformResult::NullLabel => transition_data.execution_platform = None,
        ExecutionPlatformResult::Skip => return Ok(PlannedDependency::Skipped),
        ExecutionPlatformResult::Error(message) => {
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:617 ExecGroupErrorEmitter
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:625 ExecGroupErrorEmitter.step
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:633 ExecGroupErrorEmitter.accept
            // The configuration value should already exist as a dependency so this lookup is
            // safe enough for error handling.
            let Some(configuration) = env.get_value(configuration_key)? else {
                return Ok(PlannedDependency::Missing);
            };
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:637 ExecGroupErrorEmitter.postEvent
            parameters.event_handler.post(Arc::new(
                AnalysisRootCauseEvent::with_configuration_value(
                    Some(configuration.clone()),
                    to_label.clone(),
                    message.clone(),
                ),
            ));
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
            return Ok(PlannedDependency::Error);
        }
    }
    let attribute_transition =
        match create_attribute_transition(attribute.get_transition_factory(), &transition_data) {
            Ok(t) => t,
            Err(e) => {
                errors.accept_dependency_error(DependencyError::TransitionCreation(e));
                return Ok(PlannedDependency::Error);
            }
        };
    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:212 DependencyProducer.computeTransition
    // (sink.acceptTransition(kind, toLabel, attributeTransition)).
    if let Some(collector) = &parameters.transition_collector {
        collector(kind, to_label, &attribute_transition);
    }
    let (transitioned_configurations, lookup_rounds) = match apply_transition_with_lookup_rounds(
        env,
        parameters.services,
        to_label,
        &configuration_key,
        &attribute_transition,
        parameters.event_handler,
    )? {
        None => return Ok(PlannedDependency::Missing),
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
                // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:244 DependencyProducer.acceptBuildOptionsScopeFunctionError
                TransitionApplierError::BuildOptionsScope(m) => {
                    DependencyError::BuildOptionsScope(m)
                }
                // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:249 DependencyProducer.acceptPlatformMappingError
                TransitionApplierError::PlatformMapping(m) => DependencyError::PlatformMapping(m),
                // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:254 DependencyProducer.acceptPlatformFlagsError
                TransitionApplierError::PlatformFlags(m) => DependencyError::InvalidPlatform(m),
            };
            errors.accept_dependency_error(error);
            return Ok(PlannedDependency::Error);
        }
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:224 DependencyProducer.acceptTransitionedConfigurations
    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:306 DependencyProducer.processTransitionResult
    let mut is_nonconfigurable_target_in_same_package = false;
    match get_target_in_same_package_without_skyframe(env, parameters.target(), to_label)? {
        SamePackageTarget::Missing => return Ok(PlannedDependency::Missing),
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
        return Ok(PlannedDependency::Prerequisites {
            configuration: AttributeConfiguration::NullTransitionKeys(transition_keys),
            execution_platform_label: None,
            lookup_rounds: Some(lookup_rounds),
        });
    }

    let parent_checksum = parameters
        .configuration_key()
        .expect("NullPointerException: configuration key")
        .get_options_checksum()
        .to_owned();
    for configuration in transitioned_configurations.values() {
        let child_checksum = configuration.get_options_checksum();
        if parent_checksum != child_checksum {
            parameters
                .event_handler
                .post(Arc::new(ConfigurationTransitionEvent::create(
                    &parent_checksum,
                    child_checksum,
                )));
        }
    }

    if transitioned_configurations.len() == 1 {
        if let Some(patched_configuration) = transitioned_configurations.get(PATCH_TRANSITION_KEY) {
            // It was a patch transition or no-op split transition.
            return Ok(PlannedDependency::Prerequisites {
                configuration: AttributeConfiguration::Unary(patched_configuration.clone()),
                execution_platform_label: None,
                lookup_rounds: Some(lookup_rounds),
            });
        }
    }

    Ok(PlannedDependency::Prerequisites {
        configuration: AttributeConfiguration::Split(transitioned_configurations),
        execution_platform_label: None,
        lookup_rounds: Some(lookup_rounds),
    })
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
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:374 DependencyProducer.acceptPrerequisitesValue
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:567 DependencyProducer.emitResults
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
    let base_target_prerequisites_supplier =
        if use_base_target_prerequisites_supplier(parameters, kind) {
            parameters.base_target_prerequisites_supplier
        } else {
            None
        };
    let prerequisite_values = match produce_prerequisites(
        env,
        parameters,
        to_label,
        execution_platform_label,
        configuration,
        propagating_aspects,
        base_target_prerequisites_supplier,
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
                    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:407 DependencyProducer.acceptPrerequisitesAspectError
                    PrerequisitesError::AspectEvaluation(error) => {
                        errors.accept_dependency_error(DependencyError::AspectEvaluation(error))
                    }
                    // Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:412 DependencyProducer.acceptPrerequisitesAspectError
                    PrerequisitesError::AspectCreation(error) => {
                        errors.accept_dependency_error(DependencyError::AspectCreation(error))
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

/// Returns true only during aspects evaluation for attribute dependencies not owned by an
/// aspect to enable using the `BaseTargetPrerequisitesSupplier` to look up them.
// Java: com/google/devtools/build/lib/analysis/producers/DependencyProducer.java:355 DependencyProducer.useBaseTargetPrerequisitesSupplier
fn use_base_target_prerequisites_supplier(
    parameters: &PrerequisiteParameters<'_>,
    kind: &DependencyKind,
) -> bool {
    if parameters.aspects.is_empty() {
        return false;
    }

    if kind.is_base_target_toolchain() {
        return true;
    }

    if kind.is_attribute() {
        if kind.get_owning_aspect().is_none() {
            return true;
        }
    }

    false
}

// -------------------------------------------------------------------------------------------
// PrerequisitesProducer.

/// The errors `PrerequisitesProducer.ResultSink` accepts.
enum PrerequisitesError {
    NoSuchThing(NoSuchThingException),
    /// `InvalidVisibilityDependencyException(label)`.
    InvalidVisibility(Label),
    Creation(ConfiguredValueCreationException),
    /// `acceptPrerequisitesAspectError(DependencyEvaluationException)`.
    AspectEvaluation(DependencyEvaluationException),
    /// `acceptPrerequisitesAspectError(AspectCreationException)`.
    AspectCreation(AspectCreationException),
}

/// The configured target lookups of one dependency edge (Java `PrerequisitesProducer.step` and
/// `getPrerequisiteKey`), in the order the ConfiguredTargetAndDataProducers are enqueued.
// Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:131 PrerequisitesProducer.step
fn prerequisite_requests(
    label: &Label,
    execution_platform_label: Option<&Label>,
    configuration: &AttributeConfiguration,
) -> Vec<ConfiguredTargetAndDataRequest> {
    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:350 PrerequisitesProducer.getPrerequisiteKey
    let get_prerequisite_key = |configuration_key: Option<BuildConfigurationKey>| {
        ConfiguredTargetKey::builder()
            .set_label(label.clone())
            .set_configuration_key(configuration_key)
            .set_execution_platform_label(execution_platform_label.cloned())
            .build()
    };
    match configuration {
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
    }
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
    base_target_prerequisites_supplier: Option<&BaseTargetPrerequisitesSupplier>,
) -> Produced<Result<Vec<ConfiguredTargetAndData>, Vec<PrerequisitesError>>> {
    let requests: Vec<ConfiguredTargetAndDataRequest> =
        prerequisite_requests(label, execution_platform_label.as_ref(), &configuration);
    let outcomes = produce_configured_targets_and_data(
        env,
        &requests,
        parameters.transitive_state,
        base_target_prerequisites_supplier,
    )?;

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

    // PORT: request order. Java looks the key up in the round after the configured target
    // lookups of this producer (here: after this dependency's lookups, before the next one's).
    let mut exec_aspects: Vec<Aspect> = Vec::new();
    if let Some(load_exec_aspects_key) = parameters.load_exec_aspects_key {
        // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:335 PrerequisitesProducer.acceptExecAspects
        if let Some(load_aspects_value) = env.get_value(load_exec_aspects_key.clone())? {
            exec_aspects = load_aspects_value.get_aspects().to_vec();
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:230 PrerequisitesProducer.maybeFilterAspects
    let aspects: AspectCollection = {
        // All configured targets in the set have the same underlying target so using an
        // arbitrary one for aspect filtering is safe.
        let first = &configured_targets[0];
        let filtered_aspects =
            filter_aspects_based_on_target(propagating_aspects, &exec_aspects, first);
        let computed = if filtered_aspects.is_empty() {
            Ok(AspectCollection::empty())
        } else if let Some(rule) = first.get_target().as_rule() {
            compute_aspect_collection(
                &filtered_aspects,
                rule.get_advertised_providers(),
                rule.get_label(),
                rule.get_rule_class_object(),
                &rule.get_only_tags_attribute(),
                rule.get_location(),
                parameters.event_handler,
            )
        } else {
            compute_aspect_collection_no_aspects_filtering(
                &filtered_aspects,
                first.get_target().get_label(),
                &first.get_target().get_location(),
            )
            .map_err(ComputeAspectCollectionError::InconsistentAspectOrder)
        };
        match computed {
            Ok(c) => c,
            Err(ComputeAspectCollectionError::InconsistentAspectOrder(e)) => {
                return Ok(Some(Err(vec![PrerequisitesError::AspectEvaluation(
                    DependencyEvaluationException::of_inconsistent_aspect_order(e),
                )])));
            }
            Err(ComputeAspectCollectionError::Eval(e)) => {
                parameters.event_handler.handle(&Event::error_at(
                    event_location(Some(&parameters.location())),
                    e.message_with_stack(),
                ));
                return Ok(Some(Err(vec![PrerequisitesError::AspectEvaluation(
                    DependencyEvaluationException::of_eval(
                        e.message.clone(),
                        Some(parameters.location()),
                    ),
                )])));
            }
        }
    };

    if aspects.is_empty() {
        // Short circuits if there are no aspects.
        return Ok(Some(Ok(configured_targets)));
    }

    let outcomes = produce_configured_aspects(
        env,
        &aspects,
        configured_targets,
        parameters.transitive_state,
    )?;

    // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:341 PrerequisitesProducer.emitMergedTargets
    let mut merged_targets: Vec<ConfiguredTargetAndData> = Vec::new();
    let mut errors: Vec<PrerequisitesError> = Vec::new();
    let mut missing = false;
    for outcome in outcomes {
        match outcome {
            None => missing = true,
            Some(Ok(merged)) => merged_targets.push(merged),
            Some(Err(aspect_errors)) => {
                for error in aspect_errors {
                    errors.push(match error {
                        // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:315 PrerequisitesProducer.acceptConfiguredAspectError
                        ConfiguredAspectError::Merging(e) => PrerequisitesError::AspectEvaluation(
                            DependencyEvaluationException::of_creation(
                                ConfiguredValueCreationException::new(
                                    Some(parameters.location()),
                                    e.get_message(),
                                    parameters.label(),
                                    parameters.event_id(),
                                    /* root_causes= */ None,
                                    /* detailed_exit_code= */ None,
                                ),
                                /* dep_reported_own_error= */ false,
                            ),
                        ),
                        // Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:330 PrerequisitesProducer.acceptConfiguredAspectError
                        ConfiguredAspectError::AspectCreation(e) => {
                            PrerequisitesError::AspectCreation(e)
                        }
                    });
                }
            }
        }
    }
    if !errors.is_empty() {
        return Ok(Some(Err(errors)));
    }
    if missing {
        return Ok(None);
    }
    Ok(Some(Ok(merged_targets)))
}

// Java: com/google/devtools/build/lib/analysis/producers/PrerequisitesProducer.java:287 PrerequisitesProducer.filterAspectsBasedOnTarget
fn filter_aspects_based_on_target(
    propagating_aspects: &[Aspect],
    exec_aspects: &[Aspect],
    prerequisite: &ConfiguredTargetAndData,
) -> Vec<Aspect> {
    if matches!(prerequisite.get_target(), Target::OutputFile(_)) {
        return propagating_aspects
            .iter()
            .filter(|aspect| aspect.get_definition().apply_to_generating_rules())
            .cloned()
            .collect();
    }

    if !prerequisite.get_target().is_rule() {
        return Vec::new();
    }

    if prerequisite
        .get_configuration()
        .expect("NullPointerException: rule configuration")
        .is_exec_configuration()
    {
        return propagating_aspects
            .iter()
            .chain(exec_aspects.iter())
            .cloned()
            .collect();
    }

    propagating_aspects.to_vec()
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

#[cfg(test)]
mod tests {
    use super::lookup_order_key;

    /// The labels of the CONFIGURED_TARGET keys of each request ("Group") of `node` in the
    /// oracle's `dump --skyframe=deps` excerpt of the graph-order scenario.
    fn oracle_configured_target_groups(node: &str) -> Vec<Vec<String>> {
        let dump = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/w4/scenarios/graph-order/DEPS.txt"
        ))
        .expect("scripts/w4/scenarios/graph-order/DEPS.txt");
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut in_node = false;
        for line in dump.lines() {
            if line.starts_with('#') {
                continue;
            }
            if !line.starts_with(' ') {
                in_node = line.starts_with(node);
                continue;
            }
            if !in_node {
                continue;
            }
            if line.starts_with("  Group ") {
                groups.push(Vec::new());
                continue;
            }
            let key = line.trim_start();
            if key.starts_with("CONFIGURED_TARGET:") {
                let start = key.find("label=").expect("label") + "label=".len();
                let end = key.find(", config=").expect("config");
                groups
                    .last_mut()
                    .expect("group")
                    .push(key[start..end].to_owned());
            }
        }
        groups.retain(|g| !g.is_empty());
        groups
    }

    /// Oracle: scripts/w4/scenarios/graph-order (//pkg:top). The node's `dependencyLabels` are
    /// its attributes' labels in attribute order, then the toolchain dependencies
    /// (`DependencyResolutionHelpers.visitRule`).
    #[test]
    fn configured_target_lookups_follow_the_rounds_of_the_oracle_dump() {
        let function_transition_allowlist = "@@bazel_tools//tools/allowlists/\
            function_transition_allowlist:function_transition_allowlist";
        let dependency_labels: Vec<(&str, Option<usize>)> = vec![
            // visibility: no TransitionApplier.
            ("//pkg:friends", None),
            // exec_dep (cfg = "exec"): PLATFORM, PLATFORM_MAPPING.
            ("//pkg:tool", Some(2)),
            // target_deps: PLATFORM, PLATFORM_MAPPING.
            ("//pkg:a", Some(2)),
            ("//pkg:b", Some(2)),
            // none_dep (cfg = config.none()): options without PlatformOptions, no lookup.
            ("//pkg:c", Some(0)),
            // srcs.
            ("//pkg:x.txt", Some(2)),
            ("//pkg:gen", Some(2)),
            ("//pkg:y.txt", Some(2)),
            // starlark_dep: STARLARK_BUILD_SETTINGS_DETAILS, PLATFORM, PLATFORM_MAPPING,
            // BUILD_OPTIONS_SCOPE.
            ("//pkg:d", Some(4)),
            // last_dep.
            ("//pkg:e", Some(2)),
            // none_last.
            ("//pkg:f", Some(0)),
            // _allowlist_function_transition (exec).
            (function_transition_allowlist, Some(2)),
            // The toolchain of //pkg:tt: no TransitionApplier.
            ("//pkg:tc_impl", None),
        ];
        let mut order: Vec<usize> = (0..dependency_labels.len()).collect();
        order.sort_by_key(|&i| lookup_order_key(dependency_labels[i].1, i));
        let got: Vec<&str> = order.iter().map(|&i| dependency_labels[i].0).collect();

        let groups = oracle_configured_target_groups(
            "CONFIGURED_TARGET:ConfiguredTargetKey{label=//pkg:top, \
             config=BuildConfigurationKey[a7a71fd",
        );
        // Groups 3 and 6 are the select() key and the target_compatible_with constraint; the
        // dependency map's requests follow.
        assert_eq!(groups[0], ["//pkg:is_opt"]);
        assert_eq!(groups[1], ["@@platforms//os:linux"]);
        let first_round = &groups[2];
        let same_configuration = &groups[3];
        let exec_configuration = &groups[4];
        let starlark_transition = &groups[5];
        assert_eq!(groups.len(), 6);

        // The first request: visibility and toolchain dependencies, then the attribute
        // dependencies whose transition needs no lookup.
        assert_eq!(&got[..first_round.len()], &first_round[..]);
        // The dependency whose transition needs the most rounds is last.
        assert_eq!(starlark_transition.len(), 1);
        assert_eq!(got[got.len() - 1], starlark_transition[0]);
        // In between, the dependencies whose transitions take two rounds. In this recording
        // //pkg:top is the first node to request the PLATFORM node of the execution platform:
        // the lookup had to wait, so the oracle looked the exec dependencies up one request
        // after the others (the part of the order that depends on the evaluation's timing).
        // Each of the two requests is in `dependencyLabels` order.
        let two_rounds = &got[first_round.len()..got.len() - 1];
        assert_eq!(
            two_rounds.len(),
            same_configuration.len() + exec_configuration.len()
        );
        let got_same: Vec<&str> = two_rounds
            .iter()
            .copied()
            .filter(|l| same_configuration.iter().any(|s| s == l))
            .collect();
        assert_eq!(
            got_same,
            same_configuration
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let got_exec: Vec<&str> = two_rounds
            .iter()
            .copied()
            .filter(|l| exec_configuration.iter().any(|s| s == l))
            .collect();
        assert_eq!(
            got_exec,
            exec_configuration
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }
}
