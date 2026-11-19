//! Port of `query2/query/aspectresolvers/PreciseAspectResolver.java`.

use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::Attribute;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::rule_class::AspectDetails;
use blaze_packages::target::Target;
use blaze_query_engine::InterruptedException;
use indexmap::{IndexMap, IndexSet};

use crate::pkgcache::{GetPackageError, GetSiblingsError, GetTargetError, PackageProvider};
use crate::query::aspectresolvers::aspect_resolver::{
    AspectDependencies, AspectResolver, AttributeLabels,
};
use crate::query::aspects;

/// Resolver that fully loads all direct dependencies of a target and finds out which aspects
/// apply to them (and then which dependencies the aspects add).
// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:44 PreciseAspectResolver
pub struct PreciseAspectResolver {
    package_provider: Arc<dyn PackageProvider>,
    event_handler: Arc<dyn ExtendedEventHandler>,
}

impl PreciseAspectResolver {
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:48 PreciseAspectResolver.<init>
    pub fn new(
        package_provider: Arc<dyn PackageProvider>,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Self {
        PreciseAspectResolver {
            package_provider,
            event_handler,
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:84 PreciseAspectResolver.hasDepThatSatisfies
    fn has_dep_that_satisfies(
        &self,
        aspect: &AspectDetails,
        label_deps: &[Label],
    ) -> Result<bool, InterruptedException> {
        for to_label in label_deps {
            let to_target = match self
                .package_provider
                .get_target(&self.event_handler, to_label)
            {
                Ok(t) => t,
                Err(GetTargetError::NoSuchThing(_)) => {
                    // Do nothing interesting. One of target direct deps has an error. The
                    // dependency on the BUILD file (or one of the files included in it) will be
                    // reported in the query result of :BUILD.
                    continue;
                }
                Err(GetTargetError::Interrupted(e)) => return Err(e),
            };
            let Some(rule) = to_target.as_rule() else {
                continue;
            };
            if aspects::satisfies(
                aspect,
                rule.get_rule_class_object().get_advertised_providers(),
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:106 PreciseAspectResolver.getSiblingTargets
    fn get_sibling_targets(
        &self,
        build_file: &Target,
    ) -> Result<Vec<Target>, InterruptedException> {
        match self
            .package_provider
            .get_sibling_targets_in_package(&self.event_handler, build_file)
        {
            Ok(targets) => Ok(targets),
            Err(GetSiblingsError::NoSuchPackage(_)) => {
                // If we fail to expand the full package (e.g. because a package piece for a
                // symbolic macro is in error), fall back to iterating only over the targets in
                // the BUILD file's package piece. The error encountered will be reported in the
                // eventHandler.
                blaze_util::unported!(
                    "PreciseAspectResolver.getSiblingTargets",
                    "buildFile.getPackageoid().getTargets() of a package in error"
                )
            }
            Err(GetSiblingsError::Interrupted(e)) => Err(e),
        }
    }
}

/// Java `Multimap.keySet()` of `Rule.getTransitions`: the attributes in the order of their first
/// pair, each with its labels.
fn group_by_attribute(transitions: Vec<(Attribute, Label)>) -> Vec<(Attribute, Vec<Label>)> {
    let mut grouped: IndexMap<String, (Attribute, Vec<Label>)> = IndexMap::new();
    for (attribute, label) in transitions {
        grouped
            .entry(attribute.get_name().to_owned())
            .or_insert_with(|| (attribute.clone(), Vec::new()))
            .1
            .push(label);
    }
    grouped.into_values().collect()
}

impl AspectResolver for PreciseAspectResolver {
    /// PORT: java-hash-order. `rule.getTransitions` is a `HashMultimap` keyed by `Attribute`
    /// (whose hash is a precomputed field); Blaze visits the attributes in the order
    /// blaze_packages' `Rule::get_transitions` returns them. Aspects are keyed by name, as in
    /// `ConservativeAspectResolver`.
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:54 PreciseAspectResolver.computeAspectDependencies
    fn compute_aspect_dependencies(
        &self,
        target: &Target,
        dependency_filter: &DependencyFilter,
    ) -> Result<AspectDependencies, InterruptedException> {
        let Target::Rule(rule) = target else {
            return Ok(Vec::new());
        };
        if !rule.has_aspects() {
            return Ok(Vec::new());
        }

        let mut results: IndexMap<String, (AspectDetails, AttributeLabels)> = IndexMap::new();
        let transitions =
            group_by_attribute(rule.get_transitions(&DependencyFilter::no_nodep_attributes()));
        for (attribute, labels) in &transitions {
            for aspect in aspects::get_aspects(attribute) {
                if self.has_dep_that_satisfies(aspect, labels)? {
                    let mut attribute_labels: AttributeLabels = IndexMap::new();
                    for (aspect_attribute, label) in
                        aspects::label_deps_from_all_attributes_of_aspect(aspect, dependency_filter)
                    {
                        attribute_labels
                            .entry(aspect_attribute.get_name().to_owned())
                            .or_insert_with(|| (aspect_attribute.clone(), IndexSet::new()))
                            .1
                            .insert(label);
                    }
                    if !attribute_labels.is_empty() {
                        results.insert(
                            aspect.get_name().to_owned(),
                            (aspect.clone(), attribute_labels),
                        );
                    }
                }
            }
        }
        Ok(results.into_values().collect())
    }

    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/PreciseAspectResolver.java:118 PreciseAspectResolver.computeBuildFileDependencies
    fn compute_build_file_dependencies(
        &self,
        build_file: &Target,
    ) -> Result<Vec<Label>, InterruptedException> {
        let mut result: IndexSet<Label> = IndexSet::new();
        let _ = build_file
            .get_package_declarations()
            .visit_load_graph::<()>(&mut |label| Ok(result.insert(label.clone())));

        let mut dependent_packages: IndexSet<PackageIdentifier> = IndexSet::new();
        // First compute what packages can possibly affect the aspect attributes of this package:
        // Iterate over all rules...
        for target in self.get_sibling_targets(build_file)? {
            let Target::Rule(rule) = &target else {
                continue;
            };

            // ...figure out which direct dependencies can possibly have aspects attached to
            // them...
            let deps_with_possible_aspects =
                rule.get_transitions(&DependencyFilter::new(|_info_provider, attribute| {
                    for aspect_with_parameters in aspects::get_aspects(attribute) {
                        if !aspects::aspect_definition_attributes_is_empty(aspect_with_parameters) {
                            return true;
                        }
                    }
                    false
                }));

            // ...and add the package of the aspect.
            for (_, dep_label) in &deps_with_possible_aspects {
                dependent_packages.insert(dep_label.get_package_identifier().clone());
            }
        }

        // Then add all the labels of all the bzl files loaded by the packages found.
        for package_identifier in dependent_packages {
            result.insert(Label::create_unvalidated(
                package_identifier.clone(),
                "BUILD",
            ));
            match self
                .package_provider
                .get_package(&self.event_handler, &package_identifier)
            {
                Ok(dependent_package) => {
                    let _ = dependent_package
                        .get_declarations()
                        .visit_load_graph::<()>(&mut |label| Ok(result.insert(label.clone())));
                }
                Err(GetPackageError::NoSuchPackage(_)) => {
                    // If the package is not found, just add its BUILD file, which is already done
                    // above. Hopefully this error is not raised when there is a syntax error in a
                    // subincluded file or something.
                }
                Err(GetPackageError::Interrupted(e)) => return Err(e),
            }
        }

        Ok(result.into_iter().collect())
    }
}
