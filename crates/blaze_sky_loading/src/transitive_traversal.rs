//! TRANSITIVE_TRAVERSAL: loading the transitive closure of a target without collecting it
//! (SkyQuery's universe: the graph keeps one node per target, whose direct deps are the target's
//! package and its dependencies' nodes).
//!
//! Java: com/google/devtools/build/lib/skyframe/{TransitiveTraversalFunction,
//! TransitiveTraversalValue,TransitiveBaseTraversalFunction}.java.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::aspect::aspect_definition::AspectDefinition;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchTargetException};
use blaze_packages::rule_class::advertised_provider_set::AdvertisedProviderSet;
use blaze_packages::target::Target;
use blaze_packages::target::label_visitation_utils;
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, SkyframeLookupResult,
};
use indexmap::IndexSet;

use crate::transitive_target::{LoadTargetError, TargetAndErrorIfAny, load_target};

/// Function name of TRANSITIVE_TRAVERSAL (Java `Label.TRANSITIVE_TRAVERSAL`: a `Label` is its
/// own key).
// Java: com/google/devtools/build/lib/cmdline/Label.java:108 Label.TRANSITIVE_TRAVERSAL
pub const TRANSITIVE_TRAVERSAL: SkyFunctionName = SkyFunctionName("TRANSITIVE_TRAVERSAL");

/// The key of TRANSITIVE_TRAVERSAL: the target's label. `Debug` prints the label, as Java's
/// `Label.toString`.
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:146 TransitiveTraversalValue.key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TransitiveTraversalKey(pub Label);

impl TransitiveTraversalKey {
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:146 TransitiveTraversalValue.key
    pub fn key(label: Label) -> Self {
        TransitiveTraversalKey(label)
    }

    pub fn get_label(&self) -> &Label {
        &self.0
    }
}

impl fmt::Debug for TransitiveTraversalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Key for TransitiveTraversalKey {
    type Value = TransitiveTraversalValue;
    const FUNCTION: SkyFunctionName = TRANSITIVE_TRAVERSAL;
    // Java: com/google/devtools/build/lib/cmdline/Label.java:108 Label.TRANSITIVE_TRAVERSAL (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:128 TransitiveTraversalValue.equals
    fn values_equal(old: &TransitiveTraversalValue, new: &TransitiveTraversalValue) -> bool {
        old == new
    }
}

/// The result of TRANSITIVE_TRAVERSAL: the target's kind, its rule class's advertised providers
/// and the first error found in it or its transitive dependencies.
///
/// PORT: Java interns the error-free values per target kind (`VALUES_BY_TARGET_KIND`,
/// `VALUE_INTERNER`); that only saves memory, the values are equal either way.
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:40 TransitiveTraversalValue
#[derive(Clone, PartialEq, Eq)]
pub struct TransitiveTraversalValue {
    kind: String,
    /// `Some` for `TransitiveTraversalValueWithError` (whose providers are empty).
    error_message: Option<String>,
    providers: AdvertisedProviderSet,
}

impl TransitiveTraversalValue {
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:61 TransitiveTraversalValue.unsuccessfulTransitiveTraversal
    fn unsuccessful_transitive_traversal(error_message: String, target: &Target) -> Self {
        TransitiveTraversalValue {
            kind: target.get_target_kind(),
            error_message: Some(error_message),
            providers: AdvertisedProviderSet::empty(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:67 TransitiveTraversalValue.forTarget
    fn for_target(target: &Target, error_message: Option<String>) -> Self {
        match error_message {
            None => TransitiveTraversalValue {
                kind: target.get_target_kind(),
                error_message: None,
                providers: match target.as_rule() {
                    Some(rule) => rule
                        .get_rule_class_object()
                        .get_advertised_providers()
                        .clone(),
                    None => AdvertisedProviderSet::empty(),
                },
            },
            Some(message) => TransitiveTraversalValue {
                kind: target.get_target_kind(),
                error_message: Some(message),
                providers: AdvertisedProviderSet::empty(),
            },
        }
    }

    /// Returns the set of provider names from the target, if the target is a `Rule`. Otherwise
    /// returns the empty set.
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:113 TransitiveTraversalValue.getProviders
    pub fn get_providers(&self) -> &AdvertisedProviderSet {
        &self.providers
    }

    /// Returns the target kind.
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:116 TransitiveTraversalValue.getKind
    pub fn get_kind(&self) -> &str {
        &self.kind
    }

    /// Returns the first error message, if any, from loading the target and its transitive
    /// dependencies.
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:125 TransitiveTraversalValue.getErrorMessage
    pub fn get_error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:172 TransitiveTraversalValue.TransitiveTraversalValueWithoutError.toString
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalValue.java:172 TransitiveTraversalValue.TransitiveTraversalValueWithError.toString
impl fmt::Debug for TransitiveTraversalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.error_message {
            None => write!(
                f,
                "TransitiveTraversalValueWithoutError{{kind={}, providers={:?}}}",
                self.kind, self.providers
            ),
            Some(error) => write!(
                f,
                "TransitiveTraversalValueWithError{{error={}, kind={}}}",
                error, self.kind
            ),
        }
    }
}

/// Keeps track of the first error message encountered while traversing itself and its
/// dependencies.
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:170 TransitiveTraversalFunction.FirstErrorMessageAccumulator
struct FirstErrorMessageAccumulator {
    first_error_message: Option<String>,
}

impl FirstErrorMessageAccumulator {
    /// Remembers `error_message` if it is the first error message.
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:179 TransitiveTraversalFunction.FirstErrorMessageAccumulator.maybeSet
    fn maybe_set(&mut self, error_message: String) {
        if self.first_error_message.is_none() {
            self.first_error_message = Some(error_message);
        }
    }
}

/// The message of a dependency's `NoSuchPackageException` or `NoSuchTargetException`.
fn no_such_thing_message(e: &blaze_skyframe::SkyError) -> Option<String> {
    if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
        Some(e.get_message())
    } else {
        e.downcast_ref::<NoSuchTargetException>()
            .map(|e| e.get_message().to_owned())
    }
}

/// TRANSITIVE_TRAVERSAL (Java `TransitiveTraversalFunction` over
/// `TransitiveBaseTraversalFunction`).
// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:92 TransitiveBaseTraversalFunction.compute
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:44 TransitiveTraversalFunction.argumentFromKey
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:49 TransitiveTraversalFunction.getKey
// (the key's label and `TransitiveTraversalKey::key`)
pub fn transitive_traversal_function(
    key: &TransitiveTraversalKey,
    env: &mut Environment<'_>,
) -> Result<Option<TransitiveTraversalValue>, ComputeError> {
    let label = key.get_label();
    let target_and_error_if_any = match load_target(env, label)? {
        Err(LoadTargetError::NoSuchTarget(e)) => {
            return Err(SkyFunctionException::persistent(e).into());
        }
        Err(LoadTargetError::NoSuchPackage(e)) => {
            return Err(SkyFunctionException::persistent(e).into());
        }
        Ok(None) => return Ok(None),
        Ok(Some(t)) => t,
    };

    // Process deps from attributes. It is essential that the last getValue(s) call we made to
    // skyframe for building this node was for the corresponding PackageValue.
    let label_dep_keys = transitive_traversal_get_label_dep_keys(env, &target_and_error_if_any);
    let dep_map = env.get_values_and_exceptions(&label_dep_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    // Process deps from aspects. It is essential that the second-to-last getValue(s) call we
    // made to skyframe for building this node was for the corresponding PackageValue.
    let label_aspect_keys = transitive_traversal_get_strict_label_aspect_dep_keys(
        env,
        &dep_map,
        &target_and_error_if_any,
    );
    if env.values_missing() {
        return Ok(None);
    }
    let label_aspect_entries = env.get_values_and_exceptions(&label_aspect_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut processed_targets = process_target(&target_and_error_if_any);
    process_deps(&mut processed_targets, &dep_map, &label_dep_keys);
    process_deps(
        &mut processed_targets,
        &label_aspect_entries,
        &label_aspect_keys,
    );

    // Java's getValuesAndExceptions marks missing deps eagerly (checked above); Blaze's lookup
    // result marks them when read, so check again after reading every entry.
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(compute_sky_value(
        &target_and_error_if_any,
        processed_targets,
    )))
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:109 TransitiveTraversalFunction.getLabelDepKeys
fn transitive_traversal_get_label_dep_keys(
    env: &Environment<'_>,
    target_and_error_if_any: &TargetAndErrorIfAny,
) -> Vec<SkyKey> {
    // As a performance optimization we may already know the deps we are  about to request from
    // last time #compute was called. By requesting these from the environment, we can avoid
    // repeating the label visitation step. For TransitiveTraversalFunction#compute, the label deps
    // dependency group is requested immediately after the package.
    //
    // IMPORTANT: No other package values should be requested inside
    // TransitiveTraversalFunction#compute from this point forward.
    match get_deps_after_last_package_dep(env, /* offset= */ 1) {
        Some(old_dep_keys) => old_dep_keys,
        None => get_label_dep_keys(target_and_error_if_any),
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:124 TransitiveTraversalFunction.getStrictLabelAspectDepKeys
fn transitive_traversal_get_strict_label_aspect_dep_keys(
    env: &Environment<'_>,
    dep_map: &SkyframeLookupResult,
    target_and_error_if_any: &TargetAndErrorIfAny,
) -> Vec<SkyKey> {
    // As a performance optimization we may already know the deps we are  about to request from
    // last time #compute was called. By requesting these from the environment, we can avoid
    // repeating the label visitation step. For TransitiveTraversalFunction#compute, the label
    // aspect deps dependency group is requested two groups after the package.
    match get_deps_after_last_package_dep(env, /* offset= */ 2) {
        Some(old_aspect_dep_keys) => old_aspect_dep_keys,
        None => get_strict_label_aspect_keys(&target_and_error_if_any.target, dep_map),
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:140 TransitiveTraversalFunction.getDepsAfterLastPackageDep
fn get_deps_after_last_package_dep(env: &Environment<'_>, offset: usize) -> Option<Vec<SkyKey>> {
    let temporary_direct_deps = env.get_temporary_direct_deps();
    let last_package_dep_index = get_last_package_value_index(temporary_direct_deps)?;
    if temporary_direct_deps.num_groups() <= last_package_dep_index + offset {
        return None;
    }
    Some(
        temporary_direct_deps
            .get_dep_group(last_package_dep_index + offset)
            .to_vec(),
    )
}

/// The index of the last dep group that is a single PACKAGE key (Java's `-1` is `None`).
// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:155 TransitiveTraversalFunction.getLastPackageValueIndex
fn get_last_package_value_index(direct_deps: &blaze_skyframe::GroupedDeps) -> Option<usize> {
    (0..direct_deps.num_groups()).rev().find(|&i| {
        let dep_group = direct_deps.get_dep_group(i);
        dep_group.len() == 1 && dep_group[0].function_name() == crate::sky_functions::PACKAGE
    })
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:139 TransitiveBaseTraversalFunction.getLabelDepKeys
fn get_label_dep_keys(target_and_error_if_any: &TargetAndErrorIfAny) -> Vec<SkyKey> {
    let mut deps: IndexSet<TransitiveTraversalKey> = IndexSet::new();
    label_visitation_utils::visit_target(
        &target_and_error_if_any.target,
        &DependencyFilter::no_nodep_attributes_except_visibility(),
        &mut |_from_target, _attribute, to_label| {
            deps.insert(TransitiveTraversalKey::key(to_label.clone()));
        },
    );
    deps.into_iter().map(SkyKey::new).collect()
}

/// Return the keys corresponding to the aspect-related dependencies of `target`.
// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:169 TransitiveBaseTraversalFunction.getStrictLabelAspectKeys
fn get_strict_label_aspect_keys(target: &Target, dep_map: &SkyframeLookupResult) -> Vec<SkyKey> {
    let Some(rule) = target.as_rule() else {
        // Aspects can be declared only for Rules.
        return Vec::new();
    };
    if !rule.has_aspects() {
        return Vec::new();
    }
    let mut dep_keys: Vec<SkyKey> = Vec::new();
    // Java's Multimap<Attribute, Label>: the labels of each attribute, attributes in first-seen
    // order (PORT: java-hash-order, Java's is a HashMultimap; the keys requested are the same).
    let transitions = rule.get_transitions(&DependencyFilter::no_nodep_attributes());
    let mut attributes: Vec<&blaze_packages::attribute::Attribute> = Vec::new();
    for (attribute, _) in &transitions {
        if !attributes
            .iter()
            .any(|a| a.get_name() == attribute.get_name())
        {
            attributes.push(attribute);
        }
    }
    for attribute in attributes {
        let labels: Vec<&Label> = transitions
            .iter()
            .filter(|(a, _)| a.get_name() == attribute.get_name())
            .map(|(_, l)| l)
            .collect();
        for aspect in attribute.get_aspects(rule) {
            if has_dep_that_satisfies(&aspect, &labels, dep_map) {
                AspectDefinition::for_each_label_dep_from_all_attributes_of_aspect(
                    &aspect,
                    &DependencyFilter::all_deps(),
                    &mut |_aspect_attribute, aspect_label| {
                        dep_keys.push(SkyKey::new(TransitiveTraversalKey::key(
                            aspect_label.clone(),
                        )));
                    },
                );
            }
        }
    }
    dep_keys
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:200 TransitiveBaseTraversalFunction.hasDepThatSatisfies
fn has_dep_that_satisfies(
    aspect: &blaze_packages::aspect::Aspect,
    dep_labels: &[&Label],
    full_dep_map: &SkyframeLookupResult,
) -> bool {
    for dep_label in dep_labels {
        let key = TransitiveTraversalKey::key((*dep_label).clone());
        let to_val = match full_dep_map.get_or_throw_with(&key, no_such_thing_message) {
            Err(_) => continue,
            Ok(v) => v,
        };
        // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:92 TransitiveTraversalFunction.getAdvertisedProviderSet
        if let Some(to_val) = to_val
            && AspectDefinition::satisfies(aspect, to_val.get_providers())
        {
            return true;
        }
    }
    false
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:55 TransitiveTraversalFunction.processTarget
fn process_target(target_and_error_if_any: &TargetAndErrorIfAny) -> FirstErrorMessageAccumulator {
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:174 TransitiveTraversalFunction.FirstErrorMessageAccumulator.<init>
    FirstErrorMessageAccumulator {
        first_error_message: target_and_error_if_any
            .error_loading_target
            .as_ref()
            .map(|e| e.get_message().to_owned()),
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:62 TransitiveTraversalFunction.processDeps
fn process_deps(
    accumulator: &mut FirstErrorMessageAccumulator,
    dep_entries: &SkyframeLookupResult,
    dep_keys: &[SkyKey],
) {
    for sky_key in dep_keys {
        let key = sky_key
            .downcast::<TransitiveTraversalKey>()
            .expect("a TRANSITIVE_TRAVERSAL key");
        let transitive_traversal_value =
            match dep_entries.get_or_throw_with(key, no_such_thing_message) {
                Err(message) => {
                    accumulator.maybe_set(message);
                    continue;
                }
                Ok(v) => v,
            };
        let Some(transitive_traversal_value) = transitive_traversal_value else {
            // Java: BugReport.sendBugReport("TransitiveTargetValue ... was missing, this should
            // never happen").
            continue;
        };
        if let Some(error_message) = transitive_traversal_value.get_error_message() {
            accumulator.maybe_set(error_message.to_owned());
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:99 TransitiveTraversalFunction.computeSkyValue
fn compute_sky_value(
    target_and_error_if_any: &TargetAndErrorIfAny,
    accumulator: FirstErrorMessageAccumulator,
) -> TransitiveTraversalValue {
    let target_loaded_successfully = target_and_error_if_any.error_loading_target.is_none();
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTraversalFunction.java:186 TransitiveTraversalFunction.FirstErrorMessageAccumulator.getFirstErrorMessage
    let error_message = accumulator.first_error_message;
    if target_loaded_successfully {
        TransitiveTraversalValue::for_target(&target_and_error_if_any.target, error_message)
    } else {
        TransitiveTraversalValue::unsuccessful_transitive_traversal(
            error_message.expect("checkNotNull(errorMessage)"),
            &target_and_error_if_any.target,
        )
    }
}

/// Shared for the visitors: an `Arc` of the value type, so the key's value can be read from a
/// walkable graph.
pub fn downcast_value(v: &blaze_skyframe::SkyValue) -> Option<Arc<TransitiveTraversalValue>> {
    v.clone().downcast::<TransitiveTraversalValue>().ok()
}
