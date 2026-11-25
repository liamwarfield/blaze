//! Port of `bazel/coverage/CoverageArgs.java`.

use std::sync::Arc;

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::{ActionOwner, Artifact, ArtifactOwner};
use blaze_collect::NestedSet;

use crate::analysis_services::AnalysisDirectories;
use crate::providers::file_provider::FilesToRunProvider;

/// A value class that holds arguments for `CoverageReportActionBuilder.CoverageHelper` methods.
// Java: com/google/devtools/build/lib/bazel/coverage/CoverageArgs.java:32 CoverageArgs
pub struct CoverageArgs<'a> {
    pub directories: &'a AnalysisDirectories,
    pub coverage_artifacts: NestedSet<Artifact>,
    pub lcov_artifact: Artifact,
    pub factory: &'a ArtifactFactory,
    pub artifact_owner: ArtifactOwner,
    pub report_generator: Arc<FilesToRunProvider>,
    pub workspace_name: String,
    pub html_report: Option<Artifact>,
    pub action_owner: ActionOwner,
}
