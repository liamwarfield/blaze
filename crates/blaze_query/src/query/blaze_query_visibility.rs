//! Port of `query2/query/BlazeQueryVisibility.java`.

use std::fmt;

use blaze_packages::target::{PackageGroupContents, Target};
use blaze_query_engine::QueryVisibility;

/// Converts from the Blaze package group visibility model to the query engine's.
// Java: com/google/devtools/build/lib/query2/query/BlazeQueryVisibility.java:24 BlazeQueryVisibility
pub struct BlazeQueryVisibility {
    package_specifications: PackageGroupContents,
}

impl BlazeQueryVisibility {
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryVisibility.java:28 BlazeQueryVisibility.<init>
    pub fn new(package_specifications: PackageGroupContents) -> Self {
        BlazeQueryVisibility {
            package_specifications,
        }
    }
}

impl QueryVisibility<Target> for BlazeQueryVisibility {
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryVisibility.java:33 BlazeQueryVisibility.contains
    fn contains(&self, target: &Target) -> bool {
        self.package_specifications
            .contains_package(target.get_label().get_package_identifier())
    }
}

impl fmt::Display for BlazeQueryVisibility {
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryVisibility.java:38 BlazeQueryVisibility.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.package_specifications, f)
    }
}
