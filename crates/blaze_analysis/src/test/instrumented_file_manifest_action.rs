//! Port of `analysis/test/InstrumentedFileManifestAction.java` (analysis half; its execution is
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
use std::sync::Arc;

use crate::rule::rule_context::RuleContext;

// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:44 InstrumentedFileManifestAction.GUID
const GUID: &str = "3833f0a3-7ea1-4d9f-b96f-66eff4c922b0";

/// Appends a description of a complete collection of artifacts to the fingerprint.
// Java: com/google/devtools/build/lib/actions/Artifacts.java:30 Artifacts.addToFingerprint
pub(crate) fn artifacts_add_to_fingerprint(fp: &mut Fingerprint, artifacts: &[Artifact]) {
    fp.add_int(artifacts.len() as i32);
    for artifact in artifacts {
        // Java: com/google/devtools/build/lib/actions/Artifacts.java:24 Artifacts.addToFingerprint
        fp.add_string(artifact.get_exec_path_string());
    }
}

/// Writes a manifest of instrumented source and metadata files.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:42 InstrumentedFileManifestAction
#[derive(Debug)]
pub struct InstrumentedFileManifestAction {
    base: AbstractAction,
    files: NestedSet<Artifact>,
}

impl InstrumentedFileManifestAction {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:49 InstrumentedFileManifestAction.<init>
    pub fn new(
        owner: ActionOwner,
        files: NestedSet<Artifact>,
        output: Artifact,
    ) -> InstrumentedFileManifestAction {
        InstrumentedFileManifestAction {
            base: new_abstract_file_write_action(
                owner,
                /* inputs= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                output,
            ),
            files,
        }
    }

    /// The manifest's content: the sorted exec paths, one per line.
    fn contents(&self) -> String {
        // Sort the exec paths before writing them out.
        let mut file_names: Vec<String> = self
            .files
            .to_list()
            .iter()
            .map(|a| a.get_exec_path_string().to_owned())
            .collect();
        file_names.sort();
        let mut out = String::new();
        for name in file_names {
            out.push_str(&name);
            out.push('\n');
        }
        out
    }

    /// Instantiates instrumented file manifest for the given target.
    ///
    /// `additional_source_files`: additional instrumented source files, as collected by the
    /// `InstrumentedFilesCollector`; `metadata_files`: `*.gcno`/`*.em` files collected by the
    /// `InstrumentedFilesCollector`. Returns the instrumented file manifest artifact.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:90 InstrumentedFileManifestAction.getInstrumentedFileManifest
    pub fn get_instrumented_file_manifest(
        rule_context: &RuleContext,
        additional_source_files: &NestedSet<Artifact>,
        metadata_files: &NestedSet<Artifact>,
    ) -> Artifact {
        let instrumented_file_manifest =
            rule_context.get_bin_artifact(&PathFragment::create(&format!(
                "{}.instrumented_files",
                rule_context.get_target().get_name()
            )));

        let mut inputs = NestedSetBuilder::stable_order();
        inputs.add_transitive(additional_source_files);
        inputs.add_transitive(metadata_files);
        rule_context.register_action(Arc::new(InstrumentedFileManifestAction::new(
            rule_context.get_action_owner(),
            inputs.build(),
            instrumented_file_manifest.clone(),
        )));

        instrumented_file_manifest
    }
}

impl Action for InstrumentedFileManifestAction {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.analysis.test.InstrumentedFileManifestAction")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:96 AbstractFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        abstract_file_write_action::MNEMONIC.to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:71 InstrumentedFileManifestAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // TODO(b/150305897): use addUUID?
        fp.add_string(GUID);
        // TODO(b/150308417): Not sorting is probably cheaper, might lead to unnecessary
        // re-execution.
        artifacts_add_to_fingerprint(fp, &self.files.to_list());
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

impl AbstractFileWriteAction for InstrumentedFileManifestAction {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestAction.java:55 InstrumentedFileManifestAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
            // new OutputStreamWriter(out, ISO_8859_1)
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

    /// `createArtifact(rootRelativePath)`: an artifact under `<outputBase>/exec/out`.
    fn create_artifact(root_relative_path: &str) -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create(PathFragment::create("/output_base/exec"), &fs);
        let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
        let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
        Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(BUILD_INFO_KEY))
    }

    fn manifest_action(file_a: bool, file_b: bool) -> InstrumentedFileManifestAction {
        let mut files = NestedSetBuilder::stable_order();
        if file_a {
            files.add(create_artifact("foo/a"));
        }
        if file_b {
            files.add(create_artifact("foo/b"));
        }
        InstrumentedFileManifestAction::new(
            system_action_owner(),
            files.build(),
            create_artifact("foo/manifest"),
        )
    }

    /// Regression test for b/28261106 (`ActionTester.runTest` over FILE_A, FILE_B: equal for the
    /// same attributes, different for different ones).
    // Java: src/test/java/com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestActionTest.java:80 InstrumentedFileManifestActionTest.testCacheKey
    #[test]
    fn test_cache_key() {
        let ctx = ActionKeyContext::new();
        let combos = [(false, false), (true, false), (false, true), (true, true)];
        let keys: Vec<String> = combos
            .iter()
            .map(|(a, b)| manifest_action(*a, *b).get_key(&ctx, None).unwrap())
            .collect();
        for (i, (a, b)) in combos.iter().enumerate() {
            assert_eq!(
                keys[i],
                manifest_action(*a, *b).get_key(&ctx, None).unwrap()
            );
            for j in i + 1..keys.len() {
                assert_ne!(keys[i], keys[j], "{i} and {j}");
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/analysis/test/InstrumentedFileManifestActionTest.java:47 InstrumentedFileManifestActionTest.testInstrumentedFileManifestConflicts
    #[test]
    #[ignore = "needs AnalysisTestCase.update with java_library under --collect_code_coverage"]
    fn test_instrumented_file_manifest_conflicts() {}

    #[test]
    fn writes_sorted_exec_paths() {
        let mut files = NestedSetBuilder::stable_order();
        files.add(create_artifact("foo/b"));
        files.add(create_artifact("foo/a"));
        let action = InstrumentedFileManifestAction::new(
            system_action_owner(),
            files.build(),
            create_artifact("foo/manifest"),
        );
        let bytes = action
            .new_deterministic_writer()
            .unwrap()
            .get_bytes()
            .unwrap();
        assert_eq!(bytes, b"out/foo/a\nout/foo/b\n");
    }
}
