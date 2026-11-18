//! Port of `packages/PackageGroup.java`.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_events::EventHandler;
use blaze_starlark::Location;

use crate::exceptions::PackageLoadingCode;
use crate::package::Package;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::target::PackageGroupContents;
use crate::target::PackageSpecification;
use crate::target::TargetData;
use crate::types::License;

/// A `package_group` target (Java `PackageGroup`).
// Java: com/google/devtools/build/lib/packages/PackageGroup.java:34 PackageGroup
#[derive(Debug, Clone)]
pub struct PackageGroup {
    contains_errors: bool,
    label: Label,
    location: Location,
    package_metadata: Arc<PackageMetadata>,
    package_declarations: Arc<PackageDeclarations>,
    package_specifications: PackageGroupContents,
    includes: Vec<Label>,
}

impl PackageGroup {
    /// Parses `package_specifications`, reporting invalid ones on `event_handler`.
    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:42 PackageGroup.<init>
    pub fn new(
        label: Label,
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
        package_specifications: &[String],
        includes: Vec<Label>,
        allow_public_private: bool,
        repo_root_means_current_repo: bool,
        event_handler: &dyn EventHandler,
        location: Location,
    ) -> Self {
        // TODO(bazel-team): Consider refactoring so constructor takes a PackageGroupContents.
        let mut packages = Vec::new();
        let mut errors_found = false;
        for package_specification in package_specifications {
            match PackageSpecification::from_string(
                &package_metadata.repository_mapping,
                label.get_repository(),
                package_specification,
                allow_public_private,
                repo_root_means_current_repo,
            ) {
                Ok(specification) => packages.push(specification),
                Err(e) => {
                    errors_found = true;
                    event_handler.handle(&Package::error(
                        Some(&location),
                        &e.0,
                        PackageLoadingCode::InvalidPackageSpecification,
                    ));
                }
            }
        }
        PackageGroup {
            contains_errors: errors_found,
            label,
            location,
            package_metadata,
            package_declarations,
            package_specifications: PackageGroupContents::create(&packages),
            includes,
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:83 PackageGroup.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.contains_errors
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:87 PackageGroup.getPackageSpecifications
    pub fn get_package_specifications(&self) -> &PackageGroupContents {
        &self.package_specifications
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:91 PackageGroup.contains
    pub fn contains(&self, pkg_id: &PackageIdentifier) -> bool {
        self.package_specifications.contains_package(pkg_id)
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:95 PackageGroup.getIncludes
    pub fn get_includes(&self) -> &[Label] {
        &self.includes
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:100 PackageGroup.getContainedPackages
    pub fn get_contained_packages(&self, include_double_slash: bool) -> Vec<String> {
        self.package_specifications
            .package_strings(include_double_slash)
    }

    pub fn get_label(&self) -> &Label {
        &self.label
    }

    pub fn get_name(&self) -> &str {
        self.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:115 PackageGroup.getLicense
    pub fn get_license(&self) -> License {
        License::no_license()
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:125 PackageGroup.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:130 PackageGroup.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:151 PackageGroup.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        None
    }

    /// Package groups are always public.
    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:156 PackageGroup.getVisibility
    pub fn get_visibility(&self) -> RuleVisibility {
        RuleVisibility::Public
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:166 PackageGroup.isConfigurable
    pub fn is_configurable(&self) -> bool {
        false
    }

    /// Java `PackageGroup.targetKind()`: `"package group"`.
    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:170 PackageGroup.targetKind
    pub fn target_kind() -> &'static str {
        "package group"
    }
}

impl fmt::Display for PackageGroup {
    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:145 PackageGroup.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", PackageGroup::target_kind(), self.label)
    }
}

impl TargetData for PackageGroup {
    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:135 PackageGroup.getTargetKind
    fn get_target_kind(&self) -> String {
        PackageGroup::target_kind().to_owned()
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroup.java:140 PackageGroup.getLocation
    fn get_location(&self) -> Location {
        self.location.clone()
    }

    fn get_label(&self) -> &Label {
        &self.label
    }
}
