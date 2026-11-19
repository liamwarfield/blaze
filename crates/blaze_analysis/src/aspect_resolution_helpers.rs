//! Port of `analysis/AspectResolutionHelpers.java`.

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_packages::aspect::{
    ALL_TOOLCHAINS, Aspect, AspectClass, AspectDefinition, AspectPropagationEdgesSupplier,
};
use blaze_packages::rule_class::{AdvertisedProviderSet, RuleClass};
use blaze_packages::target::{Rule, Target};
use blaze_starlark::{EvalError, Location};
use indexmap::IndexMap;

use crate::aspect_collection::AspectCollection;
use crate::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use crate::config::ordered_set_multimap::DependencyLabels;
use crate::dependency_kind::DependencyKind;
use crate::inconsistent_aspect_order_exception::InconsistentAspectOrderException;
use crate::starlark::starlark_aspect_propagation_context::StarlarkAspectPropagationContext;
use crate::toolchains::{ToolchainCollection, UnloadedToolchainContext};

/// Java `ImmutableMultimap<Aspect, T>` (an `ImmutableListMultimap`): keys in first-insertion
/// order, each key's values in insertion order.
pub type AspectMultimap<T> = IndexMap<Aspect, Vec<T>>;

/// The exceptions of [`compute_aspect_collection`] (Java `InconsistentAspectOrderException`,
/// `EvalException`).
#[derive(Debug, thiserror::Error)]
pub enum ComputeAspectCollectionError {
    #[error(transparent)]
    InconsistentAspectOrder(#[from] InconsistentAspectOrderException),
    #[error("{0}")]
    Eval(EvalError),
}

/// Computes the set of aspects that could be applied to a dependency.
///
/// This is composed of two parts:
///
/// 1. The aspects that are visible to this aspect being evaluated, if any. If another aspect is
///    visible on the configured target, it should also be visible on the dependencies for
///    consistency. This is the argument `aspects_path`.
/// 2. The aspects propagated by the attributes of this configured target / aspect.
///
/// The presence of an aspect here does not necessarily mean that it will be available on a
/// dependency: it can still be filtered out because it requires a provider that the configured
/// target it should be attached to it doesn't advertise. This is taken into account in
/// [`compute_aspect_collection`].
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:64 AspectResolutionHelpers.computePropagatingAspects
pub fn compute_propagating_aspects(
    kind: &DependencyKind,
    aspects_path: &[Aspect],
    computed_attribute_aspects: &AspectMultimap<String>,
    computed_toolchains_aspects: &AspectMultimap<Label>,
    rule: &Rule,
    base_target_toolchain_context: Option<&ToolchainCollection<UnloadedToolchainContext>>,
) -> Vec<Aspect> {
    if kind.is_base_target_toolchain() {
        return compute_propagating_aspects_to_toolchain_dep(
            kind,
            aspects_path,
            computed_toolchains_aspects,
            base_target_toolchain_context,
        );
    }

    let Some(attribute) = kind.get_attribute() else {
        return Vec::new();
    };
    let mut aspects_builder: Vec<Aspect> = attribute.get_aspects(rule);
    collect_propagating_aspects(
        aspects_path,
        computed_attribute_aspects,
        attribute.get_name(),
        kind.get_owning_aspect(),
        &mut aspects_builder,
    );
    aspects_builder
}

/// Compute the set of aspects propagating to the given `BaseTargetToolchainDependencyKind` based
/// on the `toolchains_aspects` of each aspect in the `aspects_path`.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:97 AspectResolutionHelpers.computePropagatingAspectsToToolchainDep
fn compute_propagating_aspects_to_toolchain_dep(
    kind: &DependencyKind,
    aspects_path: &[Aspect],
    computed_toolchains_aspects: &AspectMultimap<Label>,
    base_target_toolchain_context: Option<&ToolchainCollection<UnloadedToolchainContext>>,
) -> Vec<Aspect> {
    let DependencyKind::BaseTargetToolchain {
        exec_group_name,
        toolchain_type: kind_toolchain_type,
        ..
    } = kind
    else {
        panic!("ClassCastException: {kind:?} is not a BaseTargetToolchainDependencyKind");
    };
    let toolchain_context = base_target_toolchain_context
        .expect("NullPointerException: baseTargetToolchainContext")
        .get_toolchain_context(exec_group_name)
        .expect("NullPointerException: toolchainContext");
    let toolchain_type = toolchain_context
        .requested_label_to_toolchain_type()
        .get(kind_toolchain_type);

    // Since the label of the toolchain type can be an alias, we need to get all the labels that
    // point to the same toolchain type to compare them against the toolchain types that the
    // aspects can propagate.
    // Java: requestedLabelToToolchainType().asMultimap().inverse().get(toolchainType).
    let all_toolchain_type_labels: Vec<&Label> = toolchain_context
        .requested_label_to_toolchain_type()
        .iter()
        .filter(|(_, t)| Some(*t) == toolchain_type)
        .map(|(l, _)| l)
        .collect();

    let mut filtered_aspect_path: Vec<Aspect> = Vec::new();

    for aspect in aspects_path.iter().rev() {
        if all_toolchain_type_labels
            .iter()
            .any(|t| propagates_to_toolchain(t, aspect, computed_toolchains_aspects))
            || is_aspect_required(aspect, &filtered_aspect_path)
        {
            // Adds the aspect if it propagates to the toolchain type or it is required by an
            // aspect already in the `filteredAspectPath`.
            filtered_aspect_path.push(aspect.clone());
        }
    }
    filtered_aspect_path.reverse();

    filtered_aspect_path
}

/// Java `propagatesTo(edge, aspect, computedEdges)` for attribute names.
///
/// PORT: Java's generic method also checks `containsEntry(aspect, ALL_TOOLCHAINS)`, which is
/// never true for a `String` multimap.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:130 AspectResolutionHelpers.propagatesTo
fn propagates_to_attribute(
    edge: &str,
    aspect: &Aspect,
    computed_edges: &AspectMultimap<String>,
) -> bool {
    computed_edges
        .get(aspect)
        .is_some_and(|edges| edges.iter().any(|e| e == edge || e == "*"))
}

/// Java `propagatesTo(edge, aspect, computedEdges)` for toolchain types.
///
/// PORT: Java's generic method also checks `containsEntry(aspect, "*")`, which is never true for
/// a `Label` multimap.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:130 AspectResolutionHelpers.propagatesTo
fn propagates_to_toolchain(
    edge: &Label,
    aspect: &Aspect,
    computed_edges: &AspectMultimap<Label>,
) -> bool {
    computed_edges
        .get(aspect)
        .is_some_and(|edges| edges.iter().any(|e| e == edge || *e == *ALL_TOOLCHAINS))
}

/// Computes the way aspects should be computed for the direct dependencies.
///
/// This is done by filtering the aspects that can be propagated on any attribute according to
/// the providers advertised by direct dependencies and by creating the [`AspectCollection`] that
/// tells how to compute the final set of providers based on the interdependencies between the
/// propagating aspects.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:145 AspectResolutionHelpers.computeAspectCollection
pub fn compute_aspect_collection(
    aspects: &[Aspect],
    advertised_providers: &AdvertisedProviderSet,
    target_label: &Label,
    rule_class: &RuleClass,
    tags: &[String],
    target_location: &Location,
    event_handler: &dyn EventHandler,
) -> Result<AspectCollection, ComputeAspectCollectionError> {
    let mut filtered_aspect_path: Vec<Aspect> = Vec::new();

    for aspect in aspects.iter().rev() {
        if AspectDefinition::satisfies(aspect, advertised_providers)
            || is_aspect_required(aspect, &filtered_aspect_path)
        {
            // Considers the aspect if the target satisfies its required providers or it is
            // required by an aspect already in the `filteredAspectPath`.
            if evaluate_propagation_predicate(aspect, target_label, rule_class, tags, event_handler)
                .map_err(ComputeAspectCollectionError::Eval)?
            {
                // Only add the aspect if its propagation predicate is satisfied by the target.
                filtered_aspect_path.push(aspect.clone());
            }
        }
    }

    filtered_aspect_path.reverse();
    Ok(compute_aspect_collection_no_aspects_filtering(
        &filtered_aspect_path,
        target_label,
        target_location,
    )?)
}

// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:176 AspectResolutionHelpers.computeAspectCollectionNoAspectsFiltering
pub fn compute_aspect_collection_no_aspects_filtering(
    aspects: &[Aspect],
    target_label: &Label,
    target_location: &Location,
) -> Result<AspectCollection, InconsistentAspectOrderException> {
    AspectCollection::create(aspects).map_err(|e| {
        InconsistentAspectOrderException::new(target_label, target_location.clone(), &e)
    })
}

// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:186 AspectResolutionHelpers.evaluatePropagationPredicate
fn evaluate_propagation_predicate(
    aspect: &Aspect,
    label: &Label,
    rule_class: &RuleClass,
    tags: &[String],
    event_handler: &dyn EventHandler,
) -> Result<bool, EvalError> {
    let Some(propagation_predicate) = aspect.get_definition().get_propagation_predicate() else {
        return Ok(true);
    };
    let context = StarlarkAspectPropagationContext::create_for_propagation_predicate(
        aspect, label, rule_class, tags,
    );
    propagation_predicate.evaluate(&|module| context.to_value(module.heap()), event_handler)
}

/// Collects the aspects from `aspects_path` that need to be propagated along the attribute
/// `attribute_name`.
///
/// It can happen that some of the aspects cannot be propagated if the dependency doesn't have a
/// provider that's required by them. These will be filtered out after the rule class of the
/// dependency is known.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:213 AspectResolutionHelpers.collectPropagatingAspects
fn collect_propagating_aspects(
    aspects_path: &[Aspect],
    computed_attribute_aspects: &AspectMultimap<String>,
    attribute_name: &str,
    aspect_owning_attribute: Option<&AspectClass>,
    all_filtered_aspects: &mut Vec<Aspect>,
) {
    let mut filtered_aspects_path: Vec<Aspect> = Vec::new();

    // `aspectsPath` is ordered bottom up. Iterating backwards traverses top-down so the following
    // loop captures aspects that propagate along the given attribute and all their transitive
    // requirements.
    for aspect in aspects_path.iter().rev() {
        if Some(aspect.get_aspect_class()) == aspect_owning_attribute {
            // Do not propagate over the aspect's own attributes.
            continue;
        }
        if propagates_to_attribute(attribute_name, aspect, computed_attribute_aspects)
            || is_aspect_required(aspect, &filtered_aspects_path)
        {
            // Add the aspect if it can propagate over this `attributeName` based on its
            // attr_aspects or it is required by an aspect already in the `filteredAspectsPath`.
            filtered_aspects_path.push(aspect.clone());
        }
    }
    // Reverse filteredAspectsPath to return it to the same order as the input aspectsPath.
    filtered_aspects_path.reverse();
    all_filtered_aspects.extend(filtered_aspects_path);
}

/// Checks if `aspect` is required by any `Aspect` in `aspects_path`.
// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:244 AspectResolutionHelpers.isAspectRequired
fn is_aspect_required(aspect: &Aspect, aspects_path: &[Aspect]) -> bool {
    for existing_aspect in aspects_path {
        if existing_aspect.get_definition().requires(aspect) {
            return true;
        }
    }
    false
}

// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:253 AspectResolutionHelpers.computeAttributeAspects
pub fn compute_attribute_aspects(
    aspects: &[Aspect],
    target: &Target,
    attribute_map: Option<&ConfiguredAttributeMapper>,
    dependency_labels: &DependencyLabels,
    event_handler: &dyn EventHandler,
) -> Result<AspectMultimap<String>, EvalError> {
    let mut result: AspectMultimap<String> = IndexMap::new();
    for aspect in aspects {
        let attribute_aspects = aspect.get_definition().get_attribute_aspects();

        match (attribute_aspects, target) {
            (AspectPropagationEdgesSupplier::FixedList(s), _) => {
                put_all(&mut result, aspect, s.get_list().iter().cloned());
            }
            (AspectPropagationEdgesSupplier::Function(s), Target::Rule(rule)) => {
                let context = StarlarkAspectPropagationContext::create_for_propagation_edges(
                    aspect,
                    rule,
                    // Java passes the (nullable) map on; it is non-null for every rule
                    // (`DependencyResolutionHelpers.computeDependencyLabels`).
                    attribute_map.expect("NullPointerException: attributeMap of a rule"),
                    &dependency_label_lists(dependency_labels),
                );
                let list =
                    s.compute_list(&|module| context.to_value(module.heap()), event_handler)?;
                put_all(&mut result, aspect, list);
            }
            _ => {}
        }
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/analysis/AspectResolutionHelpers.java:279 AspectResolutionHelpers.computeToolchainsAspects
pub fn compute_toolchains_aspects(
    aspects: &[Aspect],
    target: &Target,
    attribute_map: Option<&ConfiguredAttributeMapper>,
    dependency_labels: &DependencyLabels,
    event_handler: &dyn EventHandler,
) -> Result<AspectMultimap<Label>, EvalError> {
    let mut result: AspectMultimap<Label> = IndexMap::new();
    for aspect in aspects {
        let toolchains_aspects = aspect.get_definition().get_toolchains_aspects();
        match (toolchains_aspects, target) {
            (AspectPropagationEdgesSupplier::FixedList(s), _) => {
                put_all(&mut result, aspect, s.get_list().iter().cloned());
            }
            (AspectPropagationEdgesSupplier::Function(s), Target::Rule(rule)) => {
                let context = StarlarkAspectPropagationContext::create_for_propagation_edges(
                    aspect,
                    rule,
                    // Java passes the (nullable) map on; it is non-null for every rule
                    // (`DependencyResolutionHelpers.computeDependencyLabels`).
                    attribute_map.expect("NullPointerException: attributeMap of a rule"),
                    &dependency_label_lists(dependency_labels),
                );
                let list =
                    s.compute_list(&|module| context.to_value(module.heap()), event_handler)?;
                put_all(&mut result, aspect, list);
            }
            _ => {}
        }
    }
    Ok(result)
}

/// Java `ImmutableMultimap.Builder.putAll(key, values)`.
fn put_all<T>(
    result: &mut AspectMultimap<T>,
    aspect: &Aspect,
    values: impl IntoIterator<Item = T>,
) {
    // Java: a key with no values is not added to the multimap.
    let values: Vec<T> = values.into_iter().collect();
    if values.is_empty() {
        return;
    }
    result.entry(aspect.clone()).or_default().extend(values);
}

/// The dependency labels as the propagation context takes them.
///
/// PORT: `StarlarkAspectPropagationContext::create_for_propagation_edges` (W4) takes label lists;
/// Java passes the `OrderedSetMultimap<DependencyKind, Label>` itself. The `None` entries (Java's
/// null label of a materializing attribute) are dropped here.
fn dependency_label_lists(
    dependency_labels: &DependencyLabels,
) -> IndexMap<DependencyKind, Vec<Label>> {
    dependency_labels
        .iter()
        .map(|(k, v)| (k.clone(), v.iter().flatten().cloned().collect()))
        .collect()
}
