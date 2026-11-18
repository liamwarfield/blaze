//! Port of `analysis/PrerequisiteArtifacts.java`.

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_packages::types::FileTypeSet;
use indexmap::IndexSet;

use crate::providers::file_provider::FileProvider;
use crate::rule::prerequisites_collection::PrerequisitesCollection;
use crate::rule::rule_context::{RuleContext, file_type_set_to_string};
use crate::rule::rule_error_consumer::RuleErrorConsumer;

/// Contains a sequence of prerequisite artifacts and supports filtering them by file type.
///
/// PORT: Java keeps the `RuleContext` to report filter errors; the Rust filter methods take it
/// as an argument (the list does not borrow the context).
// Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:33 PrerequisiteArtifacts
#[derive(Clone, Debug)]
pub struct PrerequisiteArtifacts {
    attribute_name: String,
    artifacts: Vec<Artifact>,
}

impl PrerequisiteArtifacts {
    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:45 PrerequisiteArtifacts.get
    pub fn get(rule_context: &RuleContext, attribute_name: &str) -> PrerequisiteArtifacts {
        let prerequisites: Vec<FileProvider> =
            rule_context.get_prerequisites_providers::<FileProvider>(attribute_name);
        if prerequisites.is_empty() {
            return PrerequisiteArtifacts {
                attribute_name: attribute_name.to_owned(),
                artifacts: Vec::new(),
            };
        }
        if prerequisites.len() == 1 {
            return PrerequisiteArtifacts {
                attribute_name: attribute_name.to_owned(),
                artifacts: prerequisites[0].get_files_to_build().to_list().to_vec(),
            };
        }
        let mut result: IndexSet<Artifact> = IndexSet::new();
        for target in &prerequisites {
            result.extend(target.get_files_to_build().to_list().iter().cloned());
        }
        PrerequisiteArtifacts {
            attribute_name: attribute_name.to_owned(),
            artifacts: result.into_iter().collect(),
        }
    }

    /// Returns the artifacts of the attribute's prerequisites as a stable-order nested set.
    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:66 PrerequisiteArtifacts.nestedSet
    pub fn nested_set(
        prerequisites_collection: &PrerequisitesCollection,
        attribute_name: &str,
    ) -> NestedSet<Artifact> {
        let mut result: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        for target in
            prerequisites_collection.get_prerequisites_providers::<FileProvider>(attribute_name)
        {
            result.add_transitive(target.get_files_to_build());
        }
        result.build()
    }

    /// Returns the artifacts this instance contains in an `ImmutableList`.
    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:76 PrerequisiteArtifacts.list
    pub fn list(&self) -> &[Artifact] {
        &self.artifacts
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:80 PrerequisiteArtifacts.filter
    fn filter_impl(
        &self,
        rule_context: &RuleContext,
        file_type: &FileTypeSet,
        errors_for_non_matching: bool,
    ) -> PrerequisiteArtifacts {
        let mut filtered = Vec::new();
        for artifact in &self.artifacts {
            if file_type.apply(artifact.get_filename()) {
                filtered.push(artifact.clone());
            } else if errors_for_non_matching {
                rule_context.attribute_error(
                    &self.attribute_name,
                    &format!(
                        "{} does not match expected type: {}",
                        artifact,
                        file_type_set_to_string(Some(file_type))
                    ),
                );
            }
        }
        PrerequisiteArtifacts {
            attribute_name: self.attribute_name.clone(),
            artifacts: filtered,
        }
    }

    /// Returns an instance that only contains artifacts which match the given file type set.
    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:101 PrerequisiteArtifacts.filter
    pub fn filter(
        &self,
        rule_context: &RuleContext,
        file_type_set: &FileTypeSet,
    ) -> PrerequisiteArtifacts {
        self.filter_impl(rule_context, file_type_set, false)
    }

    /// Returns an equivalent instance but which reports an error for each non-matching artifact.
    // Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:115 PrerequisiteArtifacts.errorsForNonMatching
    pub fn errors_for_non_matching(
        &self,
        rule_context: &RuleContext,
        file_type_set: &FileTypeSet,
    ) -> PrerequisiteArtifacts {
        self.filter_impl(rule_context, file_type_set, true)
    }
}

/// [`PrerequisiteArtifacts::nested_set`] as a free function.
// Java: com/google/devtools/build/lib/analysis/PrerequisiteArtifacts.java:66 PrerequisiteArtifacts.nestedSet
pub fn nested_set(
    prerequisites_collection: &PrerequisitesCollection,
    attribute_name: &str,
) -> NestedSet<Artifact> {
    PrerequisiteArtifacts::nested_set(prerequisites_collection, attribute_name)
}
