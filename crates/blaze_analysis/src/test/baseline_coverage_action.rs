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
