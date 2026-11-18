//! Port of the parts of `analysis/AnalysisUtils.java` analysis uses.

use blaze_cmdline::Label;
use blaze_vfs::PathFragment;

/// Returns a path fragment qualified by the rule name and unique fragment to disambiguate
/// artifacts produced from the source file appearing in multiple rules.
///
/// For example "//pkg:target" -> "pkg/&lt;fragment&gt;/target.
// Java: com/google/devtools/build/lib/analysis/AnalysisUtils.java:124 AnalysisUtils.getUniqueDirectory
pub fn get_unique_directory(
    label: &Label,
    fragment: &PathFragment,
    sibling_repository_layout: bool,
) -> PathFragment {
    label
        .get_package_identifier()
        .get_package_path(sibling_repository_layout)
        .get_relative(fragment)
        .get_relative(&PathFragment::create(label.get_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: com/google/devtools/build/lib/analysis/AnalysisUtilsTest (getUniqueDirectory).
    #[test]
    fn unique_directory() {
        let label = Label::parse_canonical_unchecked("//pkg:target");
        assert_eq!(
            get_unique_directory(&label, &PathFragment::create("_objs"), false).get_path_string(),
            "pkg/_objs/target"
        );
        let root = Label::parse_canonical_unchecked("//:charclass_test");
        assert_eq!(
            get_unique_directory(&root, &PathFragment::create("_objs"), false).get_path_string(),
            "_objs/charclass_test"
        );
    }
}
