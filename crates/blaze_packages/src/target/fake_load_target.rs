//! Port of `query2/compat/FakeLoadTarget.java`: a fake target so that query can report loaded
//! .bzl files as targets.
// PORT: move to blaze_query (query2/compat) if the Target enum ever opens up; it is a Target
// variant, so it lives with the targets.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_starlark::Location;

use crate::package::Package;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::target::TargetData;

/// A fake target for a loaded .bzl file (Java `FakeLoadTarget`).
// Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:28 FakeLoadTarget
#[derive(Debug, Clone)]
pub struct FakeLoadTarget {
    label: Label,
    /// Java `pkg.getMetadata()` of the package owning the BUILD file.
    package_metadata: Arc<PackageMetadata>,
    /// Java `pkg.getDeclarations()`.
    package_declarations: Arc<PackageDeclarations>,
}

impl FakeLoadTarget {
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:38 FakeLoadTarget.<init>
    pub fn new(label: Label, package_of_build_file: &Package) -> Self {
        FakeLoadTarget {
            label,
            package_metadata: package_of_build_file.get_metadata().clone(),
            package_declarations: package_of_build_file.get_declarations().clone(),
        }
    }

    /// Like [`FakeLoadTarget::new`], from the BUILD-file package's metadata and declarations
    /// (what Java reads from the packageoid).
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:38 FakeLoadTarget.<init>
    pub fn from_parts(
        label: Label,
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
    ) -> Self {
        FakeLoadTarget {
            label,
            package_metadata,
            package_declarations,
        }
    }

    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:58 FakeLoadTarget.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:63 FakeLoadTarget.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:84 FakeLoadTarget.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        Some(RuleVisibility::Public)
    }

    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:89 FakeLoadTarget.isConfigurable
    pub fn is_configurable(&self) -> bool {
        true
    }

    /// Java `FakeLoadTarget.targetKind()`: `"source file"`.
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:112 FakeLoadTarget.targetKind
    pub fn target_kind() -> &'static str {
        "source file"
    }
}

impl PartialEq for FakeLoadTarget {
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:104 FakeLoadTarget.equals
    fn eq(&self, other: &FakeLoadTarget) -> bool {
        self.label == other.label && Arc::ptr_eq(&self.package_metadata, &other.package_metadata)
    }
}

impl fmt::Display for FakeLoadTarget {
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:94 FakeLoadTarget.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

impl TargetData for FakeLoadTarget {
    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:68 FakeLoadTarget.getTargetKind
    fn get_target_kind(&self) -> String {
        FakeLoadTarget::target_kind().to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:79 FakeLoadTarget.getLocation
    fn get_location(&self) -> Location {
        self.package_metadata.get_build_file_location()
    }

    fn get_label(&self) -> &Label {
        &self.label
    }
}
