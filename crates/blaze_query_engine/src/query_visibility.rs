//! Port of `query2/engine/QueryVisibility.java`.

use std::fmt;

use crate::query_environment::TargetAccessor;

/// A predicate for targets included in the visibility list of a rule.
///
/// A rule's visibility is described by a set of `QueryVisibility`. Each element in the set
/// corresponds to an entry in the rule's visibility attribute, or an entry in the packages
/// attribute of an included package_group.
// Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:25 QueryVisibility
pub trait QueryVisibility<T>: fmt::Display {
    /// Returns true if the visibility specification includes the given target's package.
    // Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:28 QueryVisibility.contains
    fn contains(&self, target: &T) -> bool;
}

/// Global visibility.
// Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:36 QueryVisibility.EVERYTHING
pub struct Everything;

impl<T> QueryVisibility<T> for Everything {
    fn contains(&self, _target: &T) -> bool {
        true
    }
}

impl fmt::Display for Everything {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QueryVisibility(//visibility:public)")
    }
}

/// Global visibility.
// Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:32 QueryVisibility.everything
pub fn everything() -> Everything {
    Everything
}

/// Same-package visibility.
///
/// Targets are always visible to other targets in the same package. Additionally, targets under
/// java/ are always visible to the corresponding package in javatests/.
// Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:54 QueryVisibility.samePackage
pub fn same_package<'a, T>(
    from: &T,
    accessor: &'a dyn TargetAccessor<T>,
) -> SamePackageVisibility<'a, T> {
    SamePackageVisibility::new(from, accessor)
}

// Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:58 QueryVisibility.SamePackageVisibility
pub struct SamePackageVisibility<'a, T> {
    package_name: String,
    accessor: &'a dyn TargetAccessor<T>,
}

const JAVA_PREFIX: &str = "java/";
const JAVATESTS_PREFIX: &str = "javatests/";

impl<'a, T> SamePackageVisibility<'a, T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:66 SamePackageVisibility.<init>
    pub fn new(target: &T, accessor: &'a dyn TargetAccessor<T>) -> Self {
        SamePackageVisibility {
            package_name: accessor.get_package(target),
            accessor,
        }
    }
}

impl<T> QueryVisibility<T> for SamePackageVisibility<'_, T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:72 SamePackageVisibility.contains
    fn contains(&self, target: &T) -> bool {
        let other = self.accessor.get_package(target);
        if self.package_name == other {
            return true;
        }

        // packages in java/ are always visible from the corresponding package in javatests/
        if let Some(rest) = other.strip_prefix(JAVATESTS_PREFIX)
            && self.package_name == format!("{JAVA_PREFIX}{rest}")
        {
            return true;
        }

        false
    }
}

impl<T> fmt::Display for SamePackageVisibility<'_, T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryVisibility.java:88 SamePackageVisibility.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryVisibility(samePackage=<PACKAGE>)")
    }
}
