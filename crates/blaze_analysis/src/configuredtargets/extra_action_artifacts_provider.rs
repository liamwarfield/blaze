//! Port of `analysis/ExtraActionArtifactsProvider.java` (the parts `MergedConfiguredTarget`
//! uses).
//!
//! PORT: the class belongs with the other providers (`crate::providers`); it lives here because
//! `MergedConfiguredTarget.of` is its only ported user. Blaze creates no extra actions
//! (`--experimental_action_listener` is unported), so every merge yields [`EMPTY`].

use std::sync::LazyLock;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};

use crate::provider_map::TransitiveInfoProvider;

/// A `TransitiveInfoProvider` that creates extra actions.
///
/// PORT: Java's element type is `Artifact.DerivedArtifact`; Blaze's `Artifact` covers it.
// Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:27 ExtraActionArtifactsProvider
#[derive(Clone, Debug)]
pub struct ExtraActionArtifactsProvider {
    /// The outputs of the extra actions associated with this target.
    extra_action_artifacts: NestedSet<Artifact>,
    transitive_extra_action_artifacts: NestedSet<Artifact>,
}

/// Java `ExtraActionArtifactsProvider.EMPTY`.
// Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:28 ExtraActionArtifactsProvider.EMPTY
pub static EMPTY: LazyLock<ExtraActionArtifactsProvider> =
    LazyLock::new(|| ExtraActionArtifactsProvider {
        extra_action_artifacts: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        transitive_extra_action_artifacts: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
    });

impl ExtraActionArtifactsProvider {
    // Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:33 ExtraActionArtifactsProvider.create
    pub fn create(
        extra_action_artifacts: NestedSet<Artifact>,
        transitive_extra_action_artifacts: NestedSet<Artifact>,
    ) -> ExtraActionArtifactsProvider {
        if extra_action_artifacts.is_empty() && transitive_extra_action_artifacts.is_empty() {
            return EMPTY.clone();
        }
        ExtraActionArtifactsProvider {
            extra_action_artifacts,
            transitive_extra_action_artifacts,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:42 ExtraActionArtifactsProvider.merge
    pub fn merge(providers: &[ExtraActionArtifactsProvider]) -> ExtraActionArtifactsProvider {
        let mut artifacts: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        let mut transitive_extra_action_artifacts: NestedSetBuilder<Artifact> =
            NestedSetBuilder::stable_order();

        for provider in providers {
            artifacts.add_transitive(provider.get_extra_action_artifacts());
            transitive_extra_action_artifacts
                .add_transitive(provider.get_transitive_extra_action_artifacts());
        }
        ExtraActionArtifactsProvider::create(
            artifacts.build(),
            transitive_extra_action_artifacts.build(),
        )
    }

    /// The outputs of the extra actions associated with this target.
    // Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:70 ExtraActionArtifactsProvider.getExtraActionArtifacts
    pub fn get_extra_action_artifacts(&self) -> &NestedSet<Artifact> {
        &self.extra_action_artifacts
    }

    /// The outputs of the extra actions in the whole transitive closure.
    // Java: com/google/devtools/build/lib/analysis/ExtraActionArtifactsProvider.java:75 ExtraActionArtifactsProvider.getTransitiveExtraActionArtifacts
    pub fn get_transitive_extra_action_artifacts(&self) -> &NestedSet<Artifact> {
        &self.transitive_extra_action_artifacts
    }
}

impl TransitiveInfoProvider for ExtraActionArtifactsProvider {
    fn java_class_name(&self) -> &'static str {
        "ExtraActionArtifactsProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
