//! Ports of `analysis/PackageSpecificationProvider.java`, `analysis/TransitiveVisibilityProvider.java`
//! and the package-specification helpers of `analysis/Allowlist.java`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler};
use blaze_packages::starlark_defs::builtin_providers::PACKAGE_SPECIFICATION_INFO;
use blaze_packages::target::PackageGroup;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Demand, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::info::InfoProviderKey;
use crate::provider_map::TransitiveInfoProvider;
use crate::providers::visibility::{PackageGroupContentsRef, Visibility};
use crate::rule::target_context::TargetContext;
use crate::rule::to_event_location;

/// A provider that provides information about which packages a package group contains. It is
/// both a native provider and a Starlark info (`PackageSpecificationInfo`).
// Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:36 PackageSpecificationProvider
#[derive(Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PackageSpecificationProvider {
    #[allocative(skip)]
    package_specifications: Visibility,
}

starlark_simple_value!(PackageSpecificationProvider);

static EMPTY: LazyLock<Arc<PackageSpecificationProvider>> = LazyLock::new(|| {
    Arc::new(PackageSpecificationProvider {
        package_specifications: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
    })
});

impl PackageSpecificationProvider {
    /// Java `PackageSpecificationProvider.EMPTY`.
    // Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:45 PackageSpecificationProvider.EMPTY
    pub fn empty() -> Arc<PackageSpecificationProvider> {
        EMPTY.clone()
    }

    pub fn from_specifications(package_specifications: Visibility) -> PackageSpecificationProvider {
        PackageSpecificationProvider {
            package_specifications,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:55 PackageSpecificationProvider.create
    pub fn create(
        target_context: &TargetContext,
        package_group: &PackageGroup,
    ) -> PackageSpecificationProvider {
        PackageSpecificationProvider {
            package_specifications: Self::get_package_specifications_for(
                target_context,
                package_group,
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:65 PackageSpecificationProvider.getPackageSpecifications
    pub fn get_package_specifications(&self) -> &Visibility {
        &self.package_specifications
    }

    // Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:69 PackageSpecificationProvider.getPackageSpecifications
    fn get_package_specifications_for(
        target_context: &TargetContext,
        package_group: &PackageGroup,
    ) -> Visibility {
        let mut builder: NestedSetBuilder<PackageGroupContentsRef> =
            NestedSetBuilder::stable_order();
        for include_label in package_group.get_includes() {
            // Java: Optional.ofNullable(targetContext.getConfiguration()).
            let include = target_context
                .find_direct_prerequisite(include_label, target_context.get_configuration());
            let provider = include
                .as_ref()
                .and_then(|i| i.get_provider::<PackageSpecificationProvider>().cloned());
            match provider {
                None => {
                    target_context
                        .get_analysis_environment()
                        .get_event_handler()
                        .handle(&Event::error_at(
                            Some(to_event_location(
                                &target_context.get_target().get_location(),
                            )),
                            format!("Label '{include_label}' does not refer to a package group"),
                        ));
                    continue;
                }
                Some(p) => {
                    builder.add_transitive(p.get_package_specifications());
                }
            }
        }
        builder.add(PackageGroupContentsRef::new(
            package_group.get_package_specifications().clone(),
        ));
        builder.build()
    }

    /// Java `Allowlist.isAvailableFor(packageSpecifications, label)`.
    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:93 Allowlist.isAvailableFor
    pub fn is_available_for(package_group_contents: &Visibility, relevant_label: &Label) -> bool {
        package_group_contents.to_list().iter().any(|p| {
            p.get()
                .contains_package(relevant_label.get_package_identifier())
        })
    }
}

impl fmt::Debug for PackageSpecificationProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PackageSpecificationProvider({})",
            self.package_specifications
        )
    }
}

impl fmt::Display for PackageSpecificationProvider {
    // Java: NativeInfo.repr (StructImpl.repr with no fields): `struct()`.
    // PORT: Java's PackageSpecificationProvider has no fields; StructImpl.repr prints
    // "struct()".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct()")
    }
}

impl TransitiveInfoProvider for PackageSpecificationProvider {
    fn java_class_name(&self) -> &'static str {
        "PackageSpecificationProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/PackageSpecificationProviderApi.java:27 PackageSpecificationProviderApi
#[starlark_value(type = "PackageSpecificationInfo")]
impl<'v> StarlarkValue<'v> for PackageSpecificationProvider {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("PackageSpecificationInfo", package_specification_methods);
        Some(RES.methods())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(PACKAGE_SPECIFICATION_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn package_specification_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:89 PackageSpecificationProvider.targetInAllowlist
    fn contains<'v>(
        this: &PackageSpecificationProvider,
        #[starlark(require = pos)] target: Value<'v>,
    ) -> starlark::Result<bool> {
        let target_label = if let Some(s) = target.unpack_str() {
            Label::parse_canonical(s)
                .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{e}")))?
        } else if let Some(l) = target.downcast_ref::<LabelValue>() {
            l.label().clone()
        } else {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "expected string or label for 'target' instead of {}",
                target.get_type()
            )));
        };
        Ok(PackageSpecificationProvider::is_available_for(
            &this.package_specifications,
            &target_label,
        ))
    }
}

/// Holds the transitive visibility restrictions (Java `TransitiveVisibilityProvider`).
// Java: com/google/devtools/build/lib/analysis/TransitiveVisibilityProvider.java:27 TransitiveVisibilityProvider
#[derive(Clone, Debug)]
pub struct TransitiveVisibilityProvider {
    transitive_visibility: Vec<Arc<PackageSpecificationProvider>>,
}

impl TransitiveVisibilityProvider {
    // Java: com/google/devtools/build/lib/analysis/TransitiveVisibilityProvider.java:32 TransitiveVisibilityProvider.<init>
    pub fn new(transitive_visibility: Vec<Arc<PackageSpecificationProvider>>) -> Self {
        blaze_util::check_argument!(!transitive_visibility.is_empty());
        TransitiveVisibilityProvider {
            transitive_visibility,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveVisibilityProvider.java:41 TransitiveVisibilityProvider.getTransitiveVisibility
    pub fn get_transitive_visibility(&self) -> &[Arc<PackageSpecificationProvider>] {
        &self.transitive_visibility
    }
}

impl TransitiveInfoProvider for TransitiveVisibilityProvider {
    fn java_class_name(&self) -> &'static str {
        "TransitiveVisibilityProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Java `ImmutableSet<PackageSpecificationProvider>` semantics: identity-deduplicated.
pub fn add_unique(
    set: &mut Vec<Arc<PackageSpecificationProvider>>,
    p: Arc<PackageSpecificationProvider>,
) {
    if !set.iter().any(|x| Arc::ptr_eq(x, &p)) {
        set.push(p);
    }
}

/// Keeps the NestedSet type in scope for callers.
pub type PackageSpecifications = NestedSet<PackageGroupContentsRef>;
