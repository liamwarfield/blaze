//! Ports of `rules/genquery/GenQueryPackageProvider.java` and
//! `rules/genquery/GenQueryPackageProviderFactory.java` (the GENQUERY_SCOPE Skyframe function).

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::ExtendedEventHandler;
use blaze_packages::aspect::Aspect;
use blaze_packages::aspect::aspect_definition::AspectDefinition;
use blaze_packages::attribute::Attribute;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::exceptions::{
    NoSuchPackageException, NoSuchTargetException, NoSuchThingException,
};
use blaze_packages::package::Package;
use blaze_packages::target::label_visitation_utils;
use blaze_packages::target::{InputFile, Rule, Target};
use blaze_query::pkgcache::{
    CachingPackageLocator, GetPackageError, GetSiblingsError, GetTargetError, PackageProvider,
    TargetProvider,
};
use blaze_query_engine::InterruptedException;
use blaze_sky_loading::transitive_target::{LoadTargetError, load_target};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, FunctionHermeticity, Key, SkyFunctionException,
    SkyFunctionName,
};
use blaze_vfs::Path;
use indexmap::{IndexMap, IndexSet};

/// Provide packages and targets to the query operations using precomputed transitive closure.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:33 GenQueryPackageProvider
#[derive(Debug)]
pub struct GenQueryPackageProvider {
    pkg_map: IndexMap<PackageIdentifier, Arc<Package>>,
    label_to_target: IndexMap<Label, Target>,
}

impl GenQueryPackageProvider {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:38 GenQueryPackageProvider.<init>
    pub fn new(
        pkg_map: IndexMap<PackageIdentifier, Arc<Package>>,
        label_to_target: IndexMap<Label, Target>,
    ) -> GenQueryPackageProvider {
        GenQueryPackageProvider {
            pkg_map,
            label_to_target,
        }
    }

    /// `Predicates.in(labelToTarget.keySet())`.
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:44 GenQueryPackageProvider.getValidTargetPredicate
    pub fn get_valid_target_predicate(self: &Arc<Self>) -> Box<dyn Fn(&Label) -> bool> {
        let this = self.clone();
        Box::new(move |label: &Label| this.label_to_target.contains_key(label))
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:48 GenQueryPackageProvider.getPackage
    fn get_package_impl(
        &self,
        package_id: &PackageIdentifier,
    ) -> Result<Arc<Package>, NoSuchPackageException> {
        if let Some(pkg) = self.pkg_map.get(package_id) {
            return Ok(pkg.clone());
        }
        // Prefer to throw a checked exception on error; malformed genquery should not crash.
        Err(NoSuchPackageException::new(
            package_id.clone(),
            "is not within the scope of the query",
        ))
    }

    /// `getPackage(eventHandler, target)` for a monolithic package: the package of the target.
    ///
    /// PORT: Blaze targets hold their package's metadata, not the package; the package comes
    /// from the provider's map, which holds the package of every target it holds.
    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:64 PackageProvider.getPackage
    fn package_of(&self, target: &Target) -> Arc<Package> {
        let package_id = target.get_label().get_package_identifier();
        match self.pkg_map.get(package_id) {
            Some(pkg) => pkg.clone(),
            None => blaze_util::unported!(
                "PackageProvider.getPackage",
                "the package of {} is not in the genquery scope's package map",
                target.get_label()
            ),
        }
    }
}

impl TargetProvider for GenQueryPackageProvider {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:65 GenQueryPackageProvider.getTarget
    fn get_target(
        &self,
        _event_handler: &Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Result<Target, GetTargetError> {
        // Try to perform only one map lookup in the common case.
        if let Some(target) = self.label_to_target.get(label) {
            return Ok(target.clone());
        }
        // Prefer to throw a checked exception on error; malformed genquery should not crash.
        // Because it'd be more valuable, see if NoSuchPackageException should be thrown:
        if let Err(e) = self.get_package_impl(label.get_package_identifier()) {
            return Err(GetTargetError::NoSuchThing(NoSuchThingException::Package(
                e,
            )));
        }
        Err(GetTargetError::NoSuchThing(NoSuchThingException::Target(
            NoSuchTargetException::with_label(
                label.clone(),
                "is not within the scope of the query",
            ),
        )))
    }

    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:108 PackageProvider.getBuildFile
    fn get_build_file(&self, target: &Target) -> Result<Arc<InputFile>, InterruptedException> {
        // Monolithic package.
        Ok(self.package_of(target).get_build_file().clone())
    }

    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:147 PackageProvider.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(
        &self,
        _event_handler: &Arc<dyn ExtendedEventHandler>,
        target: &Target,
    ) -> Result<Vec<Target>, GetSiblingsError> {
        Ok(self
            .package_of(target)
            .get_targets()
            .values()
            .cloned()
            .collect())
    }
}

impl PackageProvider for GenQueryPackageProvider {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:48 GenQueryPackageProvider.getPackage
    fn get_package(
        &self,
        _event_handler: &Arc<dyn ExtendedEventHandler>,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, GetPackageError> {
        self.get_package_impl(package_name)
            .map_err(GetPackageError::NoSuchPackage)
    }
}

impl CachingPackageLocator for GenQueryPackageProvider {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:83 GenQueryPackageProvider.getBuildFileForPackage
    fn get_build_file_for_package(&self, package_id: &PackageIdentifier) -> Option<Path> {
        let pkg = self.pkg_map.get(package_id)?;
        Some(pkg.get_build_file().get_path().clone())
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProvider.java:94 GenQueryPackageProvider.getBaseNameForLoadedPackage
    fn get_base_name_for_loaded_package(&self, package_name: &PackageIdentifier) -> Option<String> {
        // TODO(b/123795023): we should have the data here but we don't have all packages for
        //  Starlark loads present here.
        let pkg = self.pkg_map.get(package_name)?;
        Some(pkg.get_build_file_label().get_name().to_owned())
    }
}

/// It can be common, due to macro expansion, that several genquery rules share the same value
/// for their scope attribute. By doing scope traversal as its own Skyframe node, a set of
/// genquery rules sharing the same scope will require only one scope traversal to occur.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:73 GenQueryPackageProviderFactory.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GenQueryScopeKey(Vec<Label>);

impl GenQueryScopeKey {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:77 GenQueryPackageProviderFactory.Key.<init>
    pub fn create(arg: &[Label]) -> GenQueryScopeKey {
        // ImmutableList.sortedCopyOf(arg)
        let mut sorted = arg.to_vec();
        sorted.sort();
        GenQueryScopeKey(sorted)
    }
}

// Java: com/google/devtools/build/skyframe/AbstractSkyKey.java:59 AbstractSkyKey.toString
impl fmt::Debug for GenQueryScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:[{}]",
            Self::FUNCTION.0,
            self.0
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Key for GenQueryScopeKey {
    type Value = GenQueryScopeValue;
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:69 GenQueryPackageProviderFactory.GENQUERY_SCOPE
    const FUNCTION: SkyFunctionName = SkyFunctionName("GENQUERY_SCOPE");
    // SkyFunctionName.createHermetic("GENQUERY_SCOPE")
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): Java's Value has no `equals`.
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:103 GenQueryPackageProviderFactory.Value
#[derive(Debug)]
pub struct GenQueryScopeValue {
    gen_query_package_provider: Arc<GenQueryPackageProvider>,
}

/// `BrokenQueryScopeException`.
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:407 GenQueryPackageProviderFactory.BrokenQueryScopeException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct BrokenQueryScopeException {
    message: String,
}

impl BrokenQueryScopeException {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:409 GenQueryPackageProviderFactory.BrokenQueryScopeException.of
    pub fn of(cause: Option<&str>) -> BrokenQueryScopeException {
        match cause {
            None => BrokenQueryScopeException {
                message: "errors were encountered while computing transitive closure of the scope"
                    .to_owned(),
            },
            Some(cause_message) => BrokenQueryScopeException {
                message: format!(
                    "errors were encountered while computing transitive closure of the scope: {}",
                    cause_message
                ),
            },
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Registers GENQUERY_SCOPE (Java: `GenQueryPackageProviderFactory.FUNCTION`, registered by
/// `BazelRulesModule`'s `getSkyFunctions`).
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.typed_function::<GenQueryScopeKey, _>(compute)
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:113 GenQueryPackageProviderFactory.Function.compute
fn compute(
    key: &GenQueryScopeKey,
    env: &mut Environment<'_>,
) -> Result<Option<GenQueryScopeValue>, ComputeError> {
    let scope = &key.0;
    let provider = match construct_package_map_impl(env, scope)? {
        Err(e) => return Err(SkyFunctionException::persistent(e).into()),
        Ok(None) => return Ok(None),
        Ok(Some(provider)) => provider,
    };
    Ok(Some(GenQueryScopeValue {
        gen_query_package_provider: Arc::new(provider),
    }))
}

/// `Ok(Ok(None))` where Java returns null (Skyframe restart).
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:162 GenQueryPackageProviderFactory.constructPackageMap
pub fn construct_package_map(
    env: &mut Environment<'_>,
    scope: &[Label],
) -> Result<
    Result<Option<Arc<GenQueryPackageProvider>>, BrokenQueryScopeException>,
    blaze_skyframe::Interrupted,
> {
    match env.get_value_or_throw::<GenQueryScopeKey, BrokenQueryScopeException>(
        GenQueryScopeKey::create(scope),
    )? {
        Err(e) => Ok(Err(e)),
        Ok(None) => Ok(Ok(None)),
        Ok(Some(value)) => Ok(Ok(Some(value.gen_query_package_provider.clone()))),
    }
}

/// The traversal state (Java `ScopeTraversal`).
///
/// PORT: Java keeps it in a `SkyKeyComputeState` across Skyframe restarts and reevaluates
/// partially (`PartialReevaluationMailbox`); Blaze's evaluator ports neither (PORTING.md,
/// "Skyframe"), so every evaluation starts FRESHLY_INITIALIZED with an empty traversal. A
/// restart then redoes the traversal, which reaches the same result.
#[derive(Default)]
struct ScopeTraversal {
    collected_packages: IndexMap<PackageIdentifier, Arc<Package>>,
    collected_targets: IndexMap<Label, Target>,
    labels_to_visit_in_later_restart: IndexMap<Label, ()>,
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:172 GenQueryPackageProviderFactory.constructPackageMapImpl
fn construct_package_map_impl(
    env: &mut Environment<'_>,
    scope: &[Label],
) -> Result<
    Result<Option<GenQueryPackageProvider>, BrokenQueryScopeException>,
    blaze_skyframe::Interrupted,
> {
    let mut traversal = ScopeTraversal::default();

    // FRESHLY_INITIALIZED: first evaluation; start from scratch.
    let mut labels_to_visit: IndexSet<Label> = scope.iter().cloned().collect();

    let no_nodep_attributes = DependencyFilter::no_nodep_attributes();
    let no_nodep_attributes_except_visibility =
        DependencyFilter::no_nodep_attributes_except_visibility();
    let all_deps = DependencyFilter::all_deps();

    while !labels_to_visit.is_empty() {
        let mut next_labels_to_visit: IndexSet<Label> = IndexSet::new();
        for label in &labels_to_visit {
            // If this is the first time label is visited, then collectedTargets will not contain
            // an entry for it. The else branch will do one of three things:
            // 1) discover that there is a problem with the label's package. If so, this throws
            //    BrokenQueryScopeException to stop this genquery evaluation.
            // 2) discover that needed package information has not been computed by Skyframe. If
            //    so, this records that label must be visited in a later Skyframe restart by
            //    adding it to labelsToVisitInLaterRestart; at that time that package information
            //    will have been computed.
            // 3) use the package information already computed by Skyframe to collect the label's
            //    target and package.
            //
            // Labels may be visited a second time. This happens if at least one of the label's
            // target is a rule with aspects and its dependency attributes' labels hadn't been
            // visited when the label was first visited. Note that this is the typical case for
            // such rules! This code ensures that all of a rule's dependency attributes' labels
            // are visited at least once before its label is visited a second time.
            let target = match traversal.collected_targets.get(label) {
                Some(t) => t.clone(),
                None => match load_target(env, label)? {
                    Ok(Some(target_and_error_if_any)) => {
                        if !target_and_error_if_any.package_loaded_successfully {
                            return Ok(Err(BrokenQueryScopeException::of(
                                target_and_error_if_any
                                    .error_loading_target
                                    .as_ref()
                                    .map(|e| e.get_message()),
                            )));
                        }
                        let target = target_and_error_if_any.target.clone();
                        traversal
                            .collected_targets
                            .insert(label.clone(), target.clone());
                        traversal.collected_packages.insert(
                            label.get_package_identifier().clone(),
                            target_and_error_if_any.package.clone(),
                        );
                        target
                    }
                    Ok(None) => {
                        traversal
                            .labels_to_visit_in_later_restart
                            .insert(label.clone(), ());
                        continue;
                    }
                    Err(LoadTargetError::NoSuchTarget(e)) => {
                        return Ok(Err(BrokenQueryScopeException::of(Some(e.get_message()))));
                    }
                    Err(LoadTargetError::NoSuchPackage(e)) => {
                        return Ok(Err(BrokenQueryScopeException::of(Some(&e.get_message()))));
                    }
                },
            };

            // attrDepConsumer
            let has_aspects = matches!(&target, Target::Rule(rule) if rule.has_aspects());
            let mut key_for_attr_dep_needing_restart = false;
            let mut attr_dep_unvisited = false;
            // HashMultimap<Attribute, Label>: iterated below only to enqueue labels, which a
            // later visit dedups.
            let mut transitions: IndexMap<String, (Attribute, Vec<Label>)> = IndexMap::new();
            label_visitation_utils::visit_target(
                &target,
                &no_nodep_attributes_except_visibility,
                &mut |from: &Target, attribute: Option<&Attribute>, to: &Label| {
                    if has_aspects && !key_for_attr_dep_needing_restart {
                        if traversal.labels_to_visit_in_later_restart.contains_key(to) {
                            key_for_attr_dep_needing_restart = true;
                            return;
                        }
                    }
                    if !traversal.collected_targets.contains_key(to) {
                        attr_dep_unvisited = true;
                        next_labels_to_visit.insert(to.clone());
                        return;
                    }

                    if has_aspects
                        && !key_for_attr_dep_needing_restart
                        && !attr_dep_unvisited
                        && let (Some(attribute), Target::Rule(rule)) = (attribute, from)
                        && no_nodep_attributes.test(&**rule, attribute)
                    {
                        transitions
                            .entry(attribute.get_name().to_owned())
                            .or_insert_with(|| (attribute.clone(), Vec::new()))
                            .1
                            .push(to.clone());
                    }
                },
            );

            if !has_aspects {
                continue;
            }

            if key_for_attr_dep_needing_restart {
                traversal
                    .labels_to_visit_in_later_restart
                    .insert(label.clone(), ());
                continue;
            } else if attr_dep_unvisited {
                // This schedules label to be visited a second time during this Skyframe restart.
                // Because the loop above scheduled its unvisited attribute deps for visitation,
                // and nextLabelsToVisit preserves insertion order, when label is visited a second
                // time, attributeDepUnvisited will be false, and its aspect deps will be
                // computable.
                next_labels_to_visit.insert(label.clone());
                continue;
            }

            let Target::Rule(rule) = &target else {
                unreachable!("has_aspects implies a rule");
            };
            for (attribute, to_labels) in transitions.values() {
                for aspect in attribute.get_aspects(rule) {
                    if has_dep_that_satisfies(
                        rule,
                        &aspect,
                        to_labels,
                        &traversal.collected_targets,
                    ) {
                        AspectDefinition::for_each_label_dep_from_all_attributes_of_aspect(
                            &aspect,
                            &all_deps,
                            &mut |_aspect_attribute: &Attribute, aspect_label: &Label| {
                                if !traversal.collected_targets.contains_key(aspect_label) {
                                    next_labels_to_visit.insert(aspect_label.clone());
                                }
                            },
                        );
                    }
                }
            }
        }
        labels_to_visit = next_labels_to_visit;
    }
    if env.values_missing() || !traversal.labels_to_visit_in_later_restart.is_empty() {
        return Ok(Ok(None));
    }

    Ok(Ok(Some(GenQueryPackageProvider::new(
        traversal.collected_packages,
        traversal.collected_targets,
    ))))
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQueryPackageProviderFactory.java:388 GenQueryPackageProviderFactory.hasDepThatSatisfies
fn has_dep_that_satisfies(
    from_rule: &Rule,
    aspect: &Aspect,
    to_labels: &[Label],
    targets: &IndexMap<Label, Target>,
) -> bool {
    for to_label in to_labels {
        let to_target = targets.get(to_label).unwrap_or_else(|| {
            panic!(
                "NullPointerException: {} dep {} should have been visited but was not",
                from_rule.get_label(),
                to_label
            )
        });
        if let Target::Rule(rule) = to_target
            && AspectDefinition::satisfies(
                aspect,
                rule.get_rule_class_object().get_advertised_providers(),
            )
        {
            return true;
        }
    }
    false
}
