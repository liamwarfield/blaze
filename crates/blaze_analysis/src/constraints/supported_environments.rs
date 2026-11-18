//! Ports of `analysis/constraints/SupportedEnvironmentsProvider.java` and
//! `analysis/constraints/SupportedEnvironments.java`.

use std::any::Any;

use blaze_cmdline::Label;
use blaze_packages::target::Target;
use blaze_starlark::Location;
use indexmap::IndexMap;

use crate::constraints::environment_collection::EnvironmentCollection;
use crate::provider_map::TransitiveInfoProvider;

/// A label and the location of its target (Java `LabelAndLocation`).
// Java: com/google/devtools/build/lib/analysis/LabelAndLocation.java:31 LabelAndLocation
#[derive(Clone, Debug, PartialEq)]
pub struct LabelAndLocation {
    pub label: Label,
    pub location: Location,
}

impl LabelAndLocation {
    // Java: com/google/devtools/build/lib/analysis/LabelAndLocation.java:37 LabelAndLocation.of
    pub fn of(target: &Target) -> LabelAndLocation {
        LabelAndLocation {
            label: target.get_label().clone(),
            location: target.get_location(),
        }
    }
}

/// The dependency that caused an environment to be removed from a rule's refined environments,
/// and the select() choice that removed it (Java
/// `SupportedEnvironmentsProvider.RemovedEnvironmentCulprit`).
// Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironmentsProvider.java:62 SupportedEnvironmentsProvider.RemovedEnvironmentCulprit
#[derive(Clone, Debug, PartialEq)]
pub struct RemovedEnvironmentCulprit {
    pub culprit: LabelAndLocation,
    pub selected_dep_for_culprit: Label,
}

impl RemovedEnvironmentCulprit {
    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironmentsProvider.java:69 SupportedEnvironmentsProvider.RemovedEnvironmentCulprit.create
    pub fn create(
        culprit: LabelAndLocation,
        selected_dep_for_culprit: Label,
    ) -> RemovedEnvironmentCulprit {
        RemovedEnvironmentCulprit {
            culprit,
            selected_dep_for_culprit,
        }
    }
}

/// A rule's static and refined supported environments (Java `SupportedEnvironments`, the only
/// implementation of the `SupportedEnvironmentsProvider` interface; the provider map keys it by
/// this type).
// Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:24 SupportedEnvironments
#[derive(Clone, Debug, PartialEq)]
pub struct SupportedEnvironments {
    static_environments: EnvironmentCollection,
    refined_environments: EnvironmentCollection,
    removed_environment_culprits: IndexMap<Label, RemovedEnvironmentCulprit>,
}

impl SupportedEnvironments {
    /// Java `SupportedEnvironments.EMPTY`.
    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:26 SupportedEnvironments.EMPTY
    pub fn empty() -> SupportedEnvironments {
        SupportedEnvironments {
            static_environments: EnvironmentCollection::empty(),
            refined_environments: EnvironmentCollection::empty(),
            removed_environment_culprits: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:30 SupportedEnvironments.create
    pub fn create(
        static_environments: EnvironmentCollection,
        refined_environments: EnvironmentCollection,
        removed_environment_culprits: IndexMap<Label, RemovedEnvironmentCulprit>,
    ) -> SupportedEnvironments {
        if static_environments.is_empty()
            && refined_environments.is_empty()
            && removed_environment_culprits.is_empty()
        {
            return SupportedEnvironments::empty();
        }
        SupportedEnvironments {
            static_environments,
            refined_environments,
            removed_environment_culprits,
        }
    }

    /// Returns the set of environments this rule supports, as defined by its attributes.
    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:55 SupportedEnvironments.getStaticEnvironments
    pub fn get_static_environments(&self) -> &EnvironmentCollection {
        &self.static_environments
    }

    /// Returns the set of environments this rule supports after select() resolution.
    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:60 SupportedEnvironments.getRefinedEnvironments
    pub fn get_refined_environments(&self) -> &EnvironmentCollection {
        &self.refined_environments
    }

    /// The reason `environment` was pruned from the refined environments, if it was.
    // Java: com/google/devtools/build/lib/analysis/constraints/SupportedEnvironments.java:65 SupportedEnvironments.getRemovedEnvironmentCulprit
    pub fn get_removed_environment_culprit(
        &self,
        environment: &Label,
    ) -> Option<&RemovedEnvironmentCulprit> {
        self.removed_environment_culprits.get(environment)
    }
}

impl TransitiveInfoProvider for SupportedEnvironments {
    fn java_class_name(&self) -> &'static str {
        "SupportedEnvironments"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
