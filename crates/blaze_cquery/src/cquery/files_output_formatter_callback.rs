//! Port of `query2/cquery/FilesOutputFormatterCallback.java`.

use blaze_actions::actions::artifact::Artifact;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::output_group_info::HIDDEN_OUTPUT_GROUP_PREFIX;
use blaze_collect::nestedset::{NestedSet, NestedSetBuilder};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};
use indexmap::IndexMap;

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// Cquery output formatter that prints the set of output files advertised by the matched
/// targets.
// Java: com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallback.java:36 FilesOutputFormatterCallback
pub struct FilesOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    /// `topLevelArtifactContext.outputGroups()` (an `ImmutableSortedSet`).
    ///
    /// PORT: Java passes the request's `TopLevelArtifactContext` to the environment; only its
    /// output groups are read here.
    top_level_artifact_context_output_groups: Vec<String>,
}

impl<'a> FilesOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallback.java:40 FilesOutputFormatterCallback.<init>
    pub fn new(base: CqueryThreadsafeCallback<'a>, output_groups: Vec<String>) -> Self {
        // Different targets may provide the same artifact, so we deduplicate the collection of
        // all results at the end (`uniquifyResults= true` in `base`).
        FilesOutputFormatterCallback {
            base,
            top_level_artifact_context_output_groups: output_groups,
        }
    }

    /// `TopLevelArtifactHelper.getAllArtifactsToBuild(target, context).getImportantArtifacts()`
    /// over the groups blaze_sky_top's `getAllArtifactsToBuild` returns (each group's artifacts
    /// as Java computes them for a requested group; `default` includes the files to build).
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:97 TopLevelArtifactHelper.ArtifactsToBuild.getImportantArtifacts
    fn important_artifacts(
        &self,
        all_groups: &IndexMap<String, NestedSet<Artifact>>,
    ) -> NestedSet<Artifact> {
        let mut important = NestedSetBuilder::stable_order();
        for output_group in &self.top_level_artifact_context_output_groups {
            let Some(results) = all_groups.get(output_group) else {
                // Ignore output groups that have no artifacts.
                continue;
            };
            if results.is_empty() {
                continue;
            }
            let is_important_group = !output_group.starts_with(HIDDEN_OUTPUT_GROUP_PREFIX);
            if is_important_group {
                important.add_transitive(results);
            }
        }
        important.build()
    }
}

/// Returns false if the build outputs provided by the target should never be shown to users.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:268 TopLevelArtifactHelper.shouldConsiderForDisplay
fn should_consider_for_display(configured_target: &ConfiguredTarget) -> bool {
    // TODO(bazel-team): this is quite ugly. Add a marker provider for this check.
    match configured_target {
        // Suppress display of source files (because we do no work to build them).
        ConfiguredTarget::InputFile(_) => false,
        // Suppress display of hidden rules
        ConfiguredTarget::Rule(rule_ct) => !rule_ct.get_rule_class_string().contains('$'),
        _ => true,
    }
}

/// Returns true if the given artifact should be shown to users as a build output.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:288 TopLevelArtifactHelper.shouldDisplay
fn should_display(artifact: &Artifact) -> bool {
    !artifact.is_source_artifact() && !artifact.is_runfiles_tree()
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for FilesOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallback.java:53 FilesOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "files".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for FilesOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for FilesOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for FilesOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallback.java:58 FilesOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            let Some(cf) = target.as_configured_target() else {
                continue;
            };
            if !should_consider_for_display(cf) && !matches!(cf, ConfiguredTarget::InputFile(_)) {
                continue;
            }

            let mut groups: Vec<IndexMap<String, NestedSet<Artifact>>> =
                vec![blaze_sky_top::execution_seam::get_all_artifacts_to_build(
                    cf,
                )];
            for aspect in self.base.accessor.get_top_level_aspects(target) {
                match aspect {
                    ConfiguredAspect::Value(aspect_value) => groups.push(
                        blaze_sky_top::execution_seam::get_all_artifacts_to_build_of_aspect(
                            &aspect_value,
                        ),
                    ),
                    _ => panic!("IllegalStateException: a top-level aspect is an AspectValue"),
                }
            }
            for all_groups in &groups {
                for artifact in self.important_artifacts(all_groups).to_list().iter() {
                    if should_display(artifact) || artifact.is_source_artifact() {
                        self.base
                            .add_result(artifact.get_exec_path_string().to_owned());
                    }
                }
            }
        }
        Ok(())
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}
