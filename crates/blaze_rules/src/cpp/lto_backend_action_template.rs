//! Port of `rules/cpp/LtoBackendActionTemplate.java`: an `ActionTemplate` that expands into one
//! `LtoBackendAction` per bitcode file of a tree artifact (ThinLTO of tree-artifact sources).

use std::any::Any;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, ActionRef, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::platform_info::PlatformInfoRef;
use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_cc_features::cc_toolchain_features::FeatureConfiguration;
use blaze_cc_features::cc_toolchain_variables::CcToolchainVariablesRef;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::handler::EventHandler;
use blaze_exec::actions::{ActionExecutionException, ActionTemplate, ActionTemplateError};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use super::cpp_file_types;
use super::lto_backend_action::{
    BitcodeFiles, LtoBackendAction, create_lto_backend_action_for_template,
};

/// An `ActionTemplate` that expands into `LtoBackendAction`s at execution time.
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:52 LtoBackendActionTemplate
pub struct LtoBackendActionTemplate {
    /// Owner, `getInputs()` (= `allInputs`) and `getOutputs()`.
    base: AbstractAction,
    build_variables: CcToolchainVariablesRef,
    additional_inputs: NestedSet<Artifact>,
    env: Arc<ActionEnvironment>,
    /// An input tree artifact containing the full bitcode. It is never null.
    full_bitcode_tree_artifact: Artifact,
    /// An input tree artifact containing ".thinlto.bc" and ".imports" files, generated together
    /// with the full bitcode; `None` when this is a shared non-lto backend.
    index_and_imports_tree_artifact: Option<Artifact>,
    /// An output tree artifact that will contain the native objects.
    object_file_tree_artifact: Artifact,
    /// The corresponding dwoFile if fission is used.
    dwo_file_tree_artifact: Option<Artifact>,
    feature_configuration: Arc<FeatureConfiguration>,
    use_pic: bool,
    bitcode_files: Option<Arc<BitcodeFiles>>,
    mandatory_inputs: NestedSet<Artifact>,
}

impl std::fmt::Debug for LtoBackendActionTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

impl LtoBackendActionTemplate {
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:101 LtoBackendActionTemplate.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index_and_imports_tree_artifact: Option<Artifact>,
        full_bitcode_tree_artifact: Artifact,
        object_file_tree_artifact: Artifact,
        dwo_file_tree_artifact: Option<Artifact>,
        feature_configuration: Arc<FeatureConfiguration>,
        additional_inputs: NestedSet<Artifact>,
        env: Arc<ActionEnvironment>,
        build_variables: CcToolchainVariablesRef,
        use_pic: bool,
        bitcode_files: Option<Arc<BitcodeFiles>>,
        action_owner: ActionOwner,
    ) -> LtoBackendActionTemplate {
        let mut mandatory_inputs_builder = NestedSetBuilder::compile_order();
        mandatory_inputs_builder
            .add(full_bitcode_tree_artifact.clone())
            .add_transitive(&additional_inputs);
        if let Some(index_and_imports) = &index_and_imports_tree_artifact {
            mandatory_inputs_builder.add(index_and_imports.clone());
        }
        let mandatory_inputs = mandatory_inputs_builder.build();
        // allInputs = mandatoryInputs; getOutputs(): the object tree, then the dwo tree.
        let mut outputs = vec![object_file_tree_artifact.clone()];
        if let Some(dwo) = &dwo_file_tree_artifact
            && !outputs.contains(dwo)
        {
            outputs.push(dwo.clone());
        }
        LtoBackendActionTemplate {
            base: AbstractAction::new(action_owner, mandatory_inputs.clone(), outputs),
            build_variables,
            additional_inputs,
            env,
            full_bitcode_tree_artifact,
            index_and_imports_tree_artifact,
            object_file_tree_artifact,
            dwo_file_tree_artifact,
            feature_configuration,
            use_pic,
            bitcode_files,
            mandatory_inputs,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:137 LtoBackendActionTemplate.pathFragmentToRelativePath
    fn path_fragment_to_relative_path(parent_path: &PathFragment, path: &PathFragment) -> String {
        path.relative_to(parent_path)
            .get_safe_path_string()
            .to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:141 LtoBackendActionTemplate.removeImportsExtension
    fn remove_imports_extension(path: &str) -> String {
        remove_extension(path).to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:145 LtoBackendActionTemplate.removeThinltoBcExtension
    fn remove_thinlto_bc_extension(path: &str) -> String {
        remove_extension(remove_extension(path)).to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:150 LtoBackendActionTemplate.generateDwoArtifact
    fn generate_dwo_artifact(
        &self,
        full_bitcode_relative_path: &str,
        artifact_owner: &ActionLookupKeyRef,
    ) -> Option<Artifact> {
        let dwo_file_tree_artifact = self.dwo_file_tree_artifact.as_ref()?;
        Some(Artifact::create_template_expansion_output(
            dwo_file_tree_artifact,
            replace_extension(&PathFragment::create(full_bitcode_relative_path), ".dwo"),
            artifact_owner.clone(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:162 LtoBackendActionTemplate.generateOutputObjArtifact
    fn generate_output_obj_artifact(
        &self,
        full_bitcode_relative_path: &str,
        artifact_owner: &ActionLookupKeyRef,
    ) -> Artifact {
        Artifact::create_template_expansion_output(
            &self.object_file_tree_artifact,
            PathFragment::create(full_bitcode_relative_path),
            artifact_owner.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:196 LtoBackendActionTemplate.generateActionsForNonLtoArtifacts
    fn generate_actions_for_non_lto_artifacts(
        &self,
        full_bitcode_tree_file_artifacts: &[Artifact],
        artifact_owner: &ActionLookupKeyRef,
    ) -> Vec<ActionRef> {
        let mut expanded_actions: Vec<ActionRef> =
            Vec::with_capacity(full_bitcode_tree_file_artifacts.len());
        let full_bitcode_parent_path = self.full_bitcode_tree_artifact.get_exec_path();
        for input_tree_file_artifact in full_bitcode_tree_file_artifacts {
            let path = input_tree_file_artifact.get_exec_path();
            let full_bitcode_relative_path =
                Self::path_fragment_to_relative_path(full_bitcode_parent_path, path);
            let obj_tree_file_artifact =
                self.generate_output_obj_artifact(&full_bitcode_relative_path, artifact_owner);
            let dwo_file_artifact =
                self.generate_dwo_artifact(&full_bitcode_relative_path, artifact_owner);
            let action = create_lto_backend_action_for_template(
                self.get_owner().clone(),
                &self.additional_inputs,
                self.env.clone(),
                &self.build_variables,
                self.feature_configuration.clone(),
                /* index= */ None,
                /* imports= */ None,
                input_tree_file_artifact,
                &obj_tree_file_artifact,
                self.bitcode_files.clone(),
                dwo_file_artifact.as_ref(),
                self.use_pic,
                /* bitcode_file_path= */ None,
                /* is_dummy_action= */ false,
            );
            expanded_actions.push(Arc::new(action));
        }
        expanded_actions
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:241 LtoBackendActionTemplate.generateActionsForLtoArtifacts
    fn generate_actions_for_lto_artifacts(
        &self,
        index_and_imports_tree_file_artifacts: &[Artifact],
        artifact_owner: &ActionLookupKeyRef,
    ) -> Result<Vec<ActionRef>, ActionExecutionException> {
        let mut expanded_actions: Vec<ActionRef> = Vec::new();
        let thinlto_bc_source_type = &cpp_file_types::LTO_INDEXING_ANALYSIS_FILE;
        let imports_type = &cpp_file_types::LTO_IMPORTS_FILE;

        let mut imports_builder: Vec<Artifact> = Vec::new();
        // ImmutableMap.Builder: insertion order, buildOrThrow on a duplicate key.
        let mut name_to_thin_lto: IndexMap<String, Artifact> = IndexMap::new();
        let index_and_import_parent_path = self
            .index_and_imports_tree_artifact
            .as_ref()
            .expect("an index and imports tree artifact")
            .get_exec_path();

        for input_tree_file_artifact in index_and_imports_tree_file_artifacts {
            let path = input_tree_file_artifact.get_exec_path();
            let is_thin_lto = thinlto_bc_source_type.matches_path(path);
            let is_import = imports_type.matches_path(path);
            if is_thin_lto {
                let thin_lto_no_extension = Self::remove_thinlto_bc_extension(
                    &Self::path_fragment_to_relative_path(index_and_import_parent_path, path),
                );
                if let Some(existing) = name_to_thin_lto.get(&thin_lto_no_extension) {
                    panic!(
                        "IllegalArgumentException: Multiple entries with same key: \
                         {thin_lto_no_extension}={input_tree_file_artifact} and \
                         {thin_lto_no_extension}={existing}"
                    );
                }
                name_to_thin_lto.insert(thin_lto_no_extension, input_tree_file_artifact.clone());
            } else if is_import {
                imports_builder.push(input_tree_file_artifact.clone());
            } else {
                let message = format!(
                    "Artifact '{}' expanded from the directory artifact '{}' is neither imports \
                     nor thinlto .",
                    input_tree_file_artifact.get_exec_path_string(),
                    // kinda wrong, should be index
                    self.full_bitcode_tree_artifact.get_exec_path_string()
                );
                return Err(ActionExecutionException::new(
                    message.clone(),
                    Some(self),
                    /* catastrophe= */ false,
                    Some(make_detailed_exit_code(&message)),
                ));
            }
        }

        // Maps each imports to a .bc file
        let imports = imports_builder;
        if imports.len() != name_to_thin_lto.len() {
            // `ImmutableList.toString` of the tree file artifacts (`Artifact.toString`).
            let listed: Vec<String> = index_and_imports_tree_file_artifacts
                .iter()
                .map(|a| a.to_string())
                .collect();
            let message = format!(
                "Either both or neither bitcodeFiles and imports files should be null. [{}] {}.",
                listed.join(", "),
                // kinda wrong, should be index
                self.full_bitcode_tree_artifact.get_exec_path_string()
            );
            return Err(ActionExecutionException::new(
                message.clone(),
                Some(self),
                /* catastrophe= */ false,
                Some(make_detailed_exit_code(&message)),
            ));
        }

        for import_file in &imports {
            let path = import_file.get_exec_path();
            // The relative path of the fullBitcodeFile with respect to the fullBitcodeTreeArtifact
            let full_bitcode_relative_path = Self::remove_imports_extension(
                &Self::path_fragment_to_relative_path(index_and_import_parent_path, path),
            );
            let thin_lto_file = name_to_thin_lto.get(&full_bitcode_relative_path);
            let full_bitcode_path = self
                .full_bitcode_tree_artifact
                .get_exec_path()
                .get_relative_str(&full_bitcode_relative_path);
            let obj_tree_file_artifact =
                self.generate_output_obj_artifact(&full_bitcode_relative_path, artifact_owner);
            let dwo_file_artifact =
                self.generate_dwo_artifact(&full_bitcode_relative_path, artifact_owner);
            let action = create_lto_backend_action_for_template(
                self.get_owner().clone(),
                &self.additional_inputs,
                self.env.clone(),
                &self.build_variables,
                self.feature_configuration.clone(),
                thin_lto_file,
                Some(import_file),
                &self.full_bitcode_tree_artifact,
                &obj_tree_file_artifact,
                self.bitcode_files.clone(),
                dwo_file_artifact.as_ref(),
                self.use_pic,
                Some(full_bitcode_path.get_path_string()),
                /* is_dummy_action= */ false,
            );
            expanded_actions.push(Arc::new(action));
        }
        Ok(expanded_actions)
    }

    /// This is a dummy action that would not work, because the bitcode file path is a directory
    /// rather than a file.
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:343 LtoBackendActionTemplate.getDummyAction
    fn get_dummy_action(&self) -> LtoBackendAction {
        create_lto_backend_action_for_template(
            self.get_owner().clone(),
            &self.additional_inputs,
            self.env.clone(),
            &self.build_variables,
            self.feature_configuration.clone(),
            self.index_and_imports_tree_artifact.as_ref(),
            self.index_and_imports_tree_artifact.as_ref(),
            &self.full_bitcode_tree_artifact,
            &self.object_file_tree_artifact,
            self.bitcode_files.clone(),
            self.dwo_file_tree_artifact.as_ref(),
            self.use_pic,
            None,
            /* is_dummy_action= */ true,
        )
    }
}

/// `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if let Some(last_slash_index) = filename.rfind('/')
        && last_slash_index > last_dot_index
    {
        return filename;
    }
    &filename[..last_dot_index]
}

/// `FileSystemUtils.replaceExtension(PathFragment, String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:173 FileSystemUtils.replaceExtension
fn replace_extension(path: &PathFragment, new_extension: &str) -> PathFragment {
    let new_name = format!(
        "{}{}",
        remove_extension(path.get_base_name()),
        new_extension
    );
    path.replace_name(&new_name)
        .unwrap_or_else(|| panic!("NullPointerException: replaceName of {path}"))
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:449 LtoBackendActionTemplate.makeDetailedExitCode
fn make_detailed_exit_code(message: &str) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::Execution(failure_details::Execution {
            code: failure_details::execution::Code::PersistentActionOutputDirectoryCreationFailure
                as i32,
        })),
    }
}

impl ActionTemplate for LtoBackendActionTemplate {
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:180 LtoBackendActionTemplate.generateActionsForInputArtifacts
    fn generate_actions_for_input_artifacts(
        &self,
        input_tree_file_artifacts: Vec<Artifact>,
        artifact_owner: ActionLookupKeyRef,
        _event_handler: &dyn EventHandler,
    ) -> Result<Vec<ActionRef>, ActionTemplateError> {
        if self.index_and_imports_tree_artifact.is_some() {
            Ok(self
                .generate_actions_for_lto_artifacts(&input_tree_file_artifacts, &artifact_owner)?)
        } else {
            Ok(self.generate_actions_for_non_lto_artifacts(
                &input_tree_file_artifacts,
                &artifact_owner,
            ))
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:364 LtoBackendActionTemplate.getInputTreeArtifacts
    fn get_input_tree_artifacts(&self) -> Vec<Artifact> {
        vec![
            self.index_and_imports_tree_artifact
                .clone()
                .unwrap_or_else(|| self.full_bitcode_tree_artifact.clone()),
        ]
    }
}

impl Action for LtoBackendActionTemplate {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.rules.cpp.LtoBackendActionTemplate")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:379 LtoBackendActionTemplate.getMnemonic
    fn get_mnemonic(&self) -> String {
        "LtoBackendActionTemplate".to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:328 LtoBackendActionTemplate.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        let dummy_action = self.get_dummy_action();
        dummy_action.compute_key(action_key_context, input_metadata_provider, fp)
    }

    /// An `ActionTemplate` is an `ActionAnalysisMetadata`, not an `ActionExecutionMetadata`.
    fn is_action_execution_metadata(&self) -> bool {
        false
    }

    /// Not an `AbstractAction`, so not an `ActionApi`.
    fn is_action_api(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:374 LtoBackendActionTemplate.isShareable
    fn is_shareable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:384 LtoBackendActionTemplate.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:389 LtoBackendActionTemplate.getInputFilesForExtraAction
    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        Some(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:395 LtoBackendActionTemplate.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:400 LtoBackendActionTemplate.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:410 LtoBackendActionTemplate.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:425 LtoBackendActionTemplate.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:430 LtoBackendActionTemplate.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:95 ActionTemplate.getPrimaryInput
    fn get_primary_input(&self) -> Option<Artifact> {
        Some(self.get_input_tree_artifacts()[0].clone())
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:105 ActionTemplate.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        IndexMap::new()
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:110 ActionTemplate.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        None
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:435 LtoBackendActionTemplate.prettyPrint
    fn pretty_print(&self) -> String {
        format!(
            "LtoBackendActionTemplate compiling {}",
            self.full_bitcode_tree_artifact.get_exec_path_string()
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:440 LtoBackendActionTemplate.describe
    fn describe(&self) -> String {
        format!(
            "Lto backend compiling all C++ files in {}",
            self.full_bitcode_tree_artifact.pretty_print()
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendActionTemplate.java:445 LtoBackendActionTemplate.toString
    fn to_string_java(&self) -> String {
        self.pretty_print()
    }
}
