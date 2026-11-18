//! Ports of `analysis/VisibilityProvider.java` and `analysis/VisibilityProviderImpl.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::target::{PackageGroupContents, PackageSpecification};

use crate::provider_map::TransitiveInfoProvider;

/// A `PackageGroupContents` as a NestedSet element (shared, compared by value).
///
/// PORT: `blaze_packages::PackageGroupContents` has value equality but no `Hash`; its string form
/// is a function of the value, so hashing it is consistent with equality.
#[derive(Clone)]
pub struct PackageGroupContentsRef(pub Arc<PackageGroupContents>);

impl PackageGroupContentsRef {
    pub fn new(contents: PackageGroupContents) -> PackageGroupContentsRef {
        PackageGroupContentsRef(Arc::new(contents))
    }

    pub fn get(&self) -> &PackageGroupContents {
        &self.0
    }
}

impl PartialEq for PackageGroupContentsRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl Eq for PackageGroupContentsRef {}

impl Hash for PackageGroupContentsRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_string().hash(state);
    }
}

impl fmt::Debug for PackageGroupContentsRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for PackageGroupContentsRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A target's visibility (Java `NestedSet<PackageGroupContents>`).
pub type Visibility = NestedSet<PackageGroupContentsRef>;

/// The data of a target's visibility (Java `VisibilityProvider.getVisibility()`), as stored in
/// `ConfiguredTargetBase`.
#[derive(Clone)]
pub struct VisibilityProviderData {
    pub visibility: Visibility,
}

impl VisibilityProviderData {
    pub fn new(visibility: Visibility) -> VisibilityProviderData {
        VisibilityProviderData { visibility }
    }
}

impl fmt::Debug for VisibilityProviderData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.visibility)
    }
}

// Java: com/google/devtools/build/lib/analysis/VisibilityProvider.java:41 VisibilityProvider.PUBLIC_VISIBILITY
static PUBLIC_VISIBILITY: LazyLock<Visibility> = LazyLock::new(|| {
    NestedSetBuilder::create(
        Order::STABLE_ORDER,
        [PackageGroupContentsRef::new(PackageGroupContents::create(
            &[PackageSpecification::everything()],
        ))],
    )
});

// Java: com/google/devtools/build/lib/analysis/VisibilityProvider.java:46 VisibilityProvider.PRIVATE_VISIBILITY
static PRIVATE_VISIBILITY: LazyLock<Visibility> =
    LazyLock::new(|| NestedSetBuilder::empty_set(Order::STABLE_ORDER));

/// Java `VisibilityProvider.PUBLIC_VISIBILITY`.
pub fn public_visibility() -> Visibility {
    PUBLIC_VISIBILITY.clone()
}

/// Java `VisibilityProvider.PRIVATE_VISIBILITY`.
pub fn private_visibility() -> Visibility {
    PRIVATE_VISIBILITY.clone()
}

/// Provider class for configured targets that have a visibility.
// Java: com/google/devtools/build/lib/analysis/VisibilityProvider.java:30 VisibilityProvider
// Java: com/google/devtools/build/lib/analysis/VisibilityProviderImpl.java:24 VisibilityProviderImpl
// PORT: Java's interface (implemented by every AbstractConfiguredTarget) and its standalone
// implementation are one struct; `crate::configuredtargets::get_visibility_provider` returns it
// for any configured target.
#[derive(Clone, Debug)]
pub struct VisibilityProvider {
    visibility: Visibility,
    is_created_in_symbolic_macro: bool,
}

impl VisibilityProvider {
    // Java: com/google/devtools/build/lib/analysis/VisibilityProviderImpl.java:28 VisibilityProviderImpl.<init>
    pub fn new(visibility: Visibility, is_created_in_symbolic_macro: bool) -> VisibilityProvider {
        VisibilityProvider {
            visibility,
            is_created_in_symbolic_macro,
        }
    }

    /// Returns the visibility specification, as determined by the `visibility` attribute and the
    /// package-level default visibility.
    // Java: com/google/devtools/build/lib/analysis/VisibilityProvider.java:64 VisibilityProvider.getVisibility
    pub fn get_visibility(&self) -> &Visibility {
        &self.visibility
    }

    /// Returns whether this target was instantiated in a symbolic macro.
    // Java: com/google/devtools/build/lib/analysis/VisibilityProvider.java:75 VisibilityProvider.isCreatedInSymbolicMacro
    pub fn is_created_in_symbolic_macro(&self) -> bool {
        self.is_created_in_symbolic_macro
    }
}

impl TransitiveInfoProvider for VisibilityProvider {
    fn java_class_name(&self) -> &'static str {
        "VisibilityProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_and_private_visibility() {
        assert_eq!(public_visibility().to_list().len(), 1);
        assert!(private_visibility().is_empty());
        // Java constants are singletons.
        assert!(public_visibility() == public_visibility());
    }
}
