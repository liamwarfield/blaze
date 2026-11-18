//! Port of `packages/DependencyFilter.java`.

use std::fmt;
use std::sync::Arc;

use crate::attribute::Attribute;
use crate::types::LabelClass;

/// Provides information about attributes to dependency filters (Java
/// `DependencyFilter.AttributeInfoProvider`).
// Java: com/google/devtools/build/lib/packages/DependencyFilter.java:59 DependencyFilter.AttributeInfoProvider
pub trait AttributeInfoProvider {
    /// True iff the value of the attribute is explicitly set in the BUILD file (even if equal to
    /// the default).
    // PORT: renamed from the Java overload `isAttributeValueExplicitlySpecified(Attribute)` so it
    // does not clash with `RuleOrMacroInstance::is_attribute_value_explicitly_specified(&str)`.
    fn is_attribute_value_explicitly_specified_for(&self, attribute: &Attribute) -> bool;
}

type FilterFn = dyn Fn(&dyn AttributeInfoProvider, &Attribute) -> bool + Send + Sync;

/// A predicate selecting the dependency attributes included in `blaze query` results (Java
/// `DependencyFilter`, a `BiPredicate<AttributeInfoProvider, Attribute>`).
// Java: com/google/devtools/build/lib/packages/DependencyFilter.java:24 DependencyFilter
#[derive(Clone)]
pub struct DependencyFilter(Arc<FilterFn>);

impl fmt::Debug for DependencyFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DependencyFilter(..)")
    }
}

impl DependencyFilter {
    pub fn new(
        f: impl Fn(&dyn AttributeInfoProvider, &Attribute) -> bool + Send + Sync + 'static,
    ) -> Self {
        DependencyFilter(Arc::new(f))
    }

    /// Java `BiPredicate.test`.
    pub fn test(&self, info_provider: &dyn AttributeInfoProvider, attribute: &Attribute) -> bool {
        (self.0)(info_provider, attribute)
    }

    /// Dependency predicate that includes all dependencies.
    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:27 DependencyFilter.ALL_DEPS
    pub fn all_deps() -> Self {
        DependencyFilter::new(|_, _| true)
    }

    /// Dependency predicate that excludes non-target dependencies.
    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:30 DependencyFilter.ONLY_TARGET_DEPS
    pub fn only_target_deps() -> Self {
        DependencyFilter::new(|_, attribute| !attribute.is_tool_dependency())
    }

    /// Dependency predicate that excludes implicit dependencies.
    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:33 DependencyFilter.NO_IMPLICIT_DEPS
    pub fn no_implicit_deps() -> Self {
        DependencyFilter::new(|info, attribute| {
            info.is_attribute_value_explicitly_specified_for(attribute)
        })
    }

    /// Excludes edges not present in the loading phase target dependency graph.
    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:39 DependencyFilter.NO_NODEP_ATTRIBUTES
    pub fn no_nodep_attributes() -> Self {
        DependencyFilter::new(|_, attribute| {
            attribute.get_type().get_label_class() != LabelClass::NondepReference
        })
    }

    /// Like [`DependencyFilter::no_nodep_attributes`], but includes `visibility` edges.
    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:47 DependencyFilter.NO_NODEP_ATTRIBUTES_EXCEPT_VISIBILITY
    pub fn no_nodep_attributes_except_visibility() -> Self {
        DependencyFilter::new(|_, attribute| {
            attribute.get_type().get_label_class() != LabelClass::NondepReference
                || attribute.get_name() == "visibility"
        })
    }

    // Java: com/google/devtools/build/lib/packages/DependencyFilter.java:53 DependencyFilter.and
    pub fn and(&self, other: &DependencyFilter) -> DependencyFilter {
        let a = self.clone();
        let b = other.clone();
        DependencyFilter::new(move |info, attribute| {
            a.test(info, attribute) && b.test(info, attribute)
        })
    }
}
