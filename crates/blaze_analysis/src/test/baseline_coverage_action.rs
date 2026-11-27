//! Port of `analysis/test/BaselineCoverageAction.java` (analysis half; its execution is
//! registered by `crate::test::test_runner_action_execution::register_executable_actions`).

use std::any::Any;
use std::io::Write;

use blaze_actions::actions::action::{AbstractAction, ComputeKeyError, InputMetadataProvider};
use blaze_actions::analysis::actions::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};
use blaze_actions::{Action, ActionKeyContext, ActionOwner, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::rule::rule_context::RuleContext;
use crate::test::instrumented_file_manifest_action::artifacts_add_to_fingerprint;

/// Generates baseline (empty) coverage for the given non-test target.
// Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:39 BaselineCoverageAction
#[derive(Debug)]
pub struct BaselineCoverageAction {
    base: AbstractAction,
    instrumented_files: NestedSet<Artifact>,
}

impl BaselineCoverageAction {
    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:42 BaselineCoverageAction.<init>
    fn new(
        owner: ActionOwner,
        instrumented_files: NestedSet<Artifact>,
        primary_output: Artifact,
    ) -> BaselineCoverageAction {
        BaselineCoverageAction {
            base: new_abstract_file_write_action(
                owner,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                primary_output,
            ),
            instrumented_files,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:49 BaselineCoverageAction.getInstrumentedFilesForTesting
    pub fn get_instrumented_files_for_testing(&self) -> &NestedSet<Artifact> {
        &self.instrumented_files
    }

    /// The file's content: an LCOV record without data for each instrumented file.
    fn contents(&self) -> String {
        let mut out = String::new();
        for file in self.instrumented_files.to_list().iter() {
            out.push_str(&format!("SF:{}\n", file.get_exec_path_string()));
            out.push_str("end_of_record\n");
        }
        out
    }

    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:80 BaselineCoverageAction.create
    pub fn create(
        rule_context: &RuleContext,
        instrumented_files: NestedSet<Artifact>,
    ) -> BaselineCoverageAction {
        // Baseline coverage artifacts will still go into "testlogs" directory.
        let coverage_data = rule_context.get_package_relative_artifact(
            &PathFragment::create(rule_context.get_target().get_name())
                .get_child("baseline_coverage.dat"),
            &rule_context.get_test_logs_directory(),
        );
        BaselineCoverageAction::new(
            rule_context.get_action_owner(),
            instrumented_files,
            coverage_data,
        )
    }
}

impl Action for BaselineCoverageAction {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.analysis.test.BaselineCoverageAction")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:54 BaselineCoverageAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "BaselineCoverage".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:59 BaselineCoverageAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // TODO(b/150305897): No UUID?
        // TODO(b/150308417): Sort?
        artifacts_add_to_fingerprint(fp, &self.instrumented_files.to_list());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }
}

impl AbstractFileWriteAction for BaselineCoverageAction {
    // Java: com/google/devtools/build/lib/analysis/test/BaselineCoverageAction.java:69 BaselineCoverageAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
            // new PrintWriter(out): the server's default charset (ISO-8859-1).
            out.write_all(&blaze_util::string_encoding::latin1_encode_lossy(
                &self.contents(),
            ))
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_actions::analysis::workspace_status_action::{BUILD_INFO_KEY, system_action_owner};
    use blaze_actions::{ActionLookupKeyRef, ArtifactRoot, RootType};
    use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

    fn create_artifact(root_relative_path: &str) -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create(PathFragment::create("/output_base/exec"), &fs);
        let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
        let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
        Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(BUILD_INFO_KEY))
    }

    /// The writer of `BaselineCoverageAction.newDeterministicWriter`: `SF:<exec path>` and
    /// `end_of_record` per file (the oracle wrote the same lines for the probe's `//:lib`, with
    /// source exec paths).
    #[test]
    fn writes_one_record_per_instrumented_file() {
        let mut files = NestedSetBuilder::stable_order();
        files.add(create_artifact("lib.cc"));
        files.add(create_artifact("lib.h"));
        let action = BaselineCoverageAction::new(
            system_action_owner(),
            files.build(),
            create_artifact("testlogs/lib/baseline_coverage.dat"),
        );
        assert_eq!(action.get_mnemonic(), "BaselineCoverage");
        let bytes = action
            .new_deterministic_writer()
            .unwrap()
            .get_bytes()
            .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "SF:out/lib.cc\nend_of_record\nSF:out/lib.h\nend_of_record\n"
        );
    }
}
