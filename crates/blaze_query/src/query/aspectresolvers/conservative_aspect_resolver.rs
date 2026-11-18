//! Port of `query2/query/aspectresolvers/ConservativeAspectResolver.java`.

use blaze_cmdline::Label;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::target::{RuleOrMacroInstance, Target};
use blaze_query_engine::InterruptedException;
use indexmap::IndexMap;

use crate::query::aspectresolvers::aspect_resolver::{
    AspectDependencies, AspectResolver, AttributeLabels,
};
use crate::query::aspects;

/// An aspect resolver that overestimates the required aspect dependencies.
///
/// Does not load any packages.
// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/ConservativeAspectResolver.java:35 ConservativeAspectResolver
pub struct ConservativeAspectResolver;

impl AspectResolver for ConservativeAspectResolver {
    /// PORT: Java's `LinkedHashMap<Aspect, ...>` is keyed by aspect equality (class and
    /// parameters); aspects are keyed by name here (see `query::aspects`).
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/ConservativeAspectResolver.java:37 ConservativeAspectResolver.computeAspectDependencies
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

        let mut results: IndexMap<
            String,
            (blaze_packages::rule_class::AspectDetails, AttributeLabels),
        > = IndexMap::new();
        for attribute in rule.get_attributes() {
            for aspect in aspects::get_aspects(attribute) {
                let mut attribute_labels: AttributeLabels = IndexMap::new();
                for (aspect_attribute, label) in
                    aspects::label_deps_from_all_attributes_of_aspect(aspect, dependency_filter)
                {
                    attribute_labels
                        .entry(aspect_attribute.get_name().to_owned())
                        .or_insert_with(|| (aspect_attribute.clone(), Default::default()))
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
        Ok(results.into_values().collect())
    }

    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/ConservativeAspectResolver.java:61 ConservativeAspectResolver.computeBuildFileDependencies
    fn compute_build_file_dependencies(
        &self,
        build_file: &Target,
    ) -> Result<Vec<Label>, InterruptedException> {
        Ok(build_file
            .get_package_declarations()
            .get_or_compute_transitively_loaded_starlark_files())
    }
}
