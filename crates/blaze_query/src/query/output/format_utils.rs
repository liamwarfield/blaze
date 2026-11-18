//! Port of `query2/query/output/FormatUtils.java`.

use std::cmp::Ordering;

use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::package::PackageMetadata;
use blaze_packages::target::Target;
use blaze_starlark::Location;
use blaze_vfs::PathFragment;

/// Given a set of query options, returns a BinaryPredicate suitable for passing to
/// `Rule.getLabels()`, `XmlOutputFormatter`, etc.
// Java: com/google/devtools/build/lib/query2/query/output/FormatUtils.java:37 FormatUtils.getDependencyFilter
pub fn get_dependency_filter(query_options: &QueryOptions) -> DependencyFilter {
    if query_options.include_tool_deps {
        return if query_options.include_implicit_deps {
            DependencyFilter::all_deps()
        } else {
            DependencyFilter::no_implicit_deps()
        };
    }
    if query_options.include_implicit_deps {
        DependencyFilter::only_target_deps()
    } else {
        DependencyFilter::no_implicit_deps().and(&DependencyFilter::only_target_deps())
    }
}

/// An ordering of Targets based on the ordering of their labels.
// Java: com/google/devtools/build/lib/query2/query/output/FormatUtils.java:50 FormatUtils.TargetOrdering
pub fn target_ordering(o1: &Target, o2: &Target) -> Ordering {
    o1.get_label().cmp(o2.get_label())
}

/// Returns the target location string, optionally relative to its package's source root
/// directory and optionally to display the location of source files.
// Java: com/google/devtools/build/lib/query2/query/output/FormatUtils.java:62 FormatUtils.getLocation
pub fn get_location(target: &Target, relative: bool) -> String {
    let mut loc = target.get_location();
    if let Target::InputFile(_) = target {
        let package_dir = target.get_package_metadata().get_package_directory();
        loc = Location {
            file: package_dir
                .as_fragment()
                .get_relative_str(target.get_name())
                .to_string(),
            line: 1,
            column: 1,
        };
    }
    if relative {
        loc = get_root_relative_location(loc, Some(&**target.get_package_metadata()));
    }
    loc.to_string()
}

/// Returns the specified location relative to the optional package's source root directory, if
/// available.
// Java: com/google/devtools/build/lib/query2/query/output/FormatUtils.java:78 FormatUtils.getRootRelativeLocation
pub fn get_root_relative_location(location: Location, base: Option<&PackageMetadata>) -> Location {
    if let Some(base) = base {
        let root = &base.source_root;
        let file = PathFragment::create(&location.file);
        if root.contains_fragment(&file) {
            let rel = root.relativize_fragment(&file);
            return Location {
                file: rel.to_string(),
                line: location.line,
                column: location.column,
            };
        }
    }
    location
}
