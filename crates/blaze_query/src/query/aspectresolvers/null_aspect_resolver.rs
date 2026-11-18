//! Port of `query2/query/aspectresolvers/NullAspectResolver.java`.

use blaze_cmdline::Label;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::target::Target;
use blaze_query_engine::InterruptedException;

use crate::query::aspectresolvers::aspect_resolver::{AspectDependencies, AspectResolver};

/// An aspect resolver that returns only those aspects that are possibly active given the rule
/// classes of direct dependencies. Does not resolve aspects.
// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/NullAspectResolver.java:29 NullAspectResolver
pub struct NullAspectResolver;

impl AspectResolver for NullAspectResolver {
    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/NullAspectResolver.java:31 NullAspectResolver.computeAspectDependencies
    fn compute_aspect_dependencies(
        &self,
        _target: &Target,
        _dependency_filter: &DependencyFilter,
    ) -> Result<AspectDependencies, InterruptedException> {
        Ok(Vec::new())
    }

    // Java: com/google/devtools/build/lib/query2/query/aspectresolvers/NullAspectResolver.java:37 NullAspectResolver.computeBuildFileDependencies
    fn compute_build_file_dependencies(
        &self,
        build_file: &Target,
    ) -> Result<Vec<Label>, InterruptedException> {
        Ok(build_file
            .get_package_declarations()
            .get_or_compute_transitively_loaded_starlark_files())
    }
}
