//! Port of `query2/query/aspectresolvers/AspectResolver.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::command::aspect_resolver::AspectResolverMode;
use blaze_packages::attribute::Attribute;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::rule_class::AspectDetails;
use blaze_packages::target::Target;
use blaze_query_engine::InterruptedException;
use indexmap::{IndexMap, IndexSet};

use crate::query::aspectresolvers::conservative_aspect_resolver::ConservativeAspectResolver;
use crate::query::aspectresolvers::null_aspect_resolver::NullAspectResolver;

/// One attribute's entries of Java's `ImmutableMultimap<Attribute, Label>` (keyed by attribute
/// name; `ImmutableSetMultimap` keeps the first insertion order of keys and values).
pub type AttributeLabels = IndexMap<String, (Attribute, IndexSet<Label>)>;

/// Java `ImmutableMap<Aspect, ImmutableMultimap<Attribute, Label>>`, in insertion order.
pub type AspectDependencies = Vec<(AspectDetails, AttributeLabels)>;

/// Utility class that determines additional dependencies of a target from its aspects.
// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java:32 AspectResolver
pub trait AspectResolver {
    /// Compute additional dependencies of target from aspects. This method may load the direct
    /// deps of target to determine their types. Returns map of attributes to corresponding label
    /// values.
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java:70 AspectResolver.computeAspectDependencies
    fn compute_aspect_dependencies(
        &self,
        target: &Target,
        dependency_filter: &DependencyFilter,
    ) -> Result<AspectDependencies, InterruptedException>;

    /// Compute the labels of the BUILD Starlark files on which the results of the other two
    /// methods depend for a target in the given package.
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java:77 AspectResolver.computeBuildFileDependencies
    fn compute_build_file_dependencies(
        &self,
        build_file: &Target,
    ) -> Result<Vec<Label>, InterruptedException>;
}

/// How to resolve aspect dependencies in query (Java `AspectResolver.Mode.createResolver`).
///
/// PORT: Java passes a `PackageProvider` and event handler (used by the precise resolver only).
// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java:37 AspectResolver.Mode.createResolver
pub fn create_resolver(mode: AspectResolverMode) -> Arc<dyn AspectResolver> {
    match mode {
        AspectResolverMode::OFF => Arc::new(NullAspectResolver),
        AspectResolverMode::CONSERVATIVE => Arc::new(ConservativeAspectResolver),
        AspectResolverMode::PRECISE => blaze_util::unported!("PreciseAspectResolver"),
    }
}
