//! Port of `analysis/actions/SpawnActionTemplate.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, ActionRef, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::platform_info::PlatformInfoRef;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_actions::analysis::actions::spawn_action::{SpawnAction, SpawnActionBuilder};
use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::handler::EventHandler;
use blaze_exec::actions::{ActionTemplate, ActionTemplateError};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::providers::file_provider::FilesToRunProvider;

/// Provides the mapping between expanded input files under the input TreeArtifact and the
/// parent-relative paths of their associated output files under the output TreeArtifact.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:74 SpawnActionTemplate.OutputPathMapper
pub type OutputPathMapper = Arc<dyn Fn(&Artifact) -> PathFragment + Send + Sync>;

/// An `ActionTemplate` that expands into `SpawnAction`s at execution time.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:54 SpawnActionTemplate
pub struct SpawnActionTemplate {
    /// Owner, `getInputs()` (= `allInputs`) and `getOutputs()`.
    base: AbstractAction,
    input_tree_artifact: Artifact,
    output_tree_artifact: Artifact,
    common_tools: NestedSet<Artifact>,
    all_inputs: NestedSet<Artifact>,
    mnemonic: String,
    output_path_mapper: OutputPathMapper,
    spawn_action_builder: SpawnActionBuilder,
    command_line_template: Arc<CustomCommandLine>,
}

impl std::fmt::Debug for SpawnActionTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

impl SpawnActionTemplate {
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:90 SpawnActionTemplate.<init>
    #[allow(clippy::too_many_arguments)]
    fn new(
        action_owner: ActionOwner,
        input_tree_artifact: Artifact,
        output_tree_artifact: Artifact,
        common_inputs: NestedSet<Artifact>,
        common_tools: NestedSet<Artifact>,
        output_path_mapper: OutputPathMapper,
        command_line_template: Arc<CustomCommandLine>,
        mnemonic: String,
        spawn_action_builder: SpawnActionBuilder,
    ) -> SpawnActionTemplate {
        let mut all_inputs = NestedSetBuilder::stable_order();
        all_inputs
            .add(input_tree_artifact.clone())
            .add_transitive(&common_inputs)
            .add_transitive(&common_tools);
        let all_inputs = all_inputs.build();
        SpawnActionTemplate {
            base: AbstractAction::new(
                action_owner,
                all_inputs.clone(),
                vec![output_tree_artifact.clone()],
            ),
            input_tree_artifact,
            output_tree_artifact,
            common_tools,
            all_inputs,
            mnemonic,
            output_path_mapper,
            spawn_action_builder,
            command_line_template,
        }
    }

    /// Returns a SpawnAction that takes `input_tree_file_artifact` as input and generates
    /// `output_tree_file_artifact`.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:153 SpawnActionTemplate.createAction
    fn create_action(
        &self,
        input_tree_file_artifact: &Artifact,
        output_tree_file_artifact: &Artifact,
    ) -> SpawnAction {
        // new ExpandedSpawnAction.Builder(spawnActionBuilder)
        let mut action_builder = SpawnActionBuilder::from_builder(&self.spawn_action_builder);
        action_builder.add_input(input_tree_file_artifact.clone());
        action_builder.add_output(output_tree_file_artifact.clone());
        let command_line = self.command_line_template.evaluate_tree_file_artifacts([
            input_tree_file_artifact.clone(),
            output_tree_file_artifact.clone(),
        ]);
        action_builder.add_command_line(Arc::new(command_line));
        // Note that we pass in nulls below because SpawnActionTemplate does not support param
        // file, and it does not use any default value for executable or shell environment. They
        // must be set explicitly via builder method #setExecutable and #setEnvironment.
        // PORT: ExpandedSpawnAction overrides getCommandLineLimits with UNLIMITED; Blaze's
        // SpawnAction has no command line limits (no param files are spilled without
        // `use_param_file`), so the expanded action is a plain SpawnAction of that class.
        action_builder
            .build_for_action_template(self.get_owner().clone())
            .with_java_class(
                "com.google.devtools.build.lib.analysis.actions.SpawnActionTemplate$ExpandedSpawnAction",
            )
    }
}

impl ActionTemplate for SpawnActionTemplate {
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:111 SpawnActionTemplate.generateActionsForInputArtifacts
    fn generate_actions_for_input_artifacts(
        &self,
        input_tree_file_artifacts: Vec<Artifact>,
        artifact_owner: ActionLookupKeyRef,
        _event_handler: &dyn EventHandler,
    ) -> Result<Vec<ActionRef>, ActionTemplateError> {
        let mut expanded_actions: Vec<ActionRef> =
            Vec::with_capacity(input_tree_file_artifacts.len());
        for input_tree_file_artifact in &input_tree_file_artifacts {
            let parent_relative_output_path = (self.output_path_mapper)(input_tree_file_artifact);
            let output_tree_file_artifact = Artifact::create_template_expansion_output(
                &self.output_tree_artifact,
                parent_relative_output_path,
                artifact_owner.clone(),
            );
            expanded_actions.push(Arc::new(
                self.create_action(input_tree_file_artifact, &output_tree_file_artifact),
            ));
        }
        Ok(expanded_actions)
    }

    /// Returns the input TreeArtifact(s).
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:177 SpawnActionTemplate.getInputTreeArtifacts
    fn get_input_tree_artifacts(&self) -> Vec<Artifact> {
        vec![self.input_tree_artifact.clone()]
    }
}

impl Action for SpawnActionTemplate {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.analysis.actions.SpawnActionTemplate")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:197 SpawnActionTemplate.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:132 SpawnActionTemplate.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        let input_tree_file_artifact =
            Artifact::create_tree_output_str(&self.input_tree_artifact, "dummy_for_key");
        let output_tree_file_artifact = Artifact::create_template_expansion_output(
            &self.output_tree_artifact,
            (self.output_path_mapper)(&input_tree_file_artifact),
            blaze_exec::actions::action_template_expansion_key(
                match self.output_tree_artifact.get_artifact_owner() {
                    blaze_actions::ArtifactOwner::ActionLookupKey(k) => k,
                    other => panic!("ClassCastException: {other:?} is not an ActionLookupKey"),
                },
                /* action_index= */ 0,
            ),
        );
        let dummy_action =
            self.create_action(&input_tree_file_artifact, &output_tree_file_artifact);
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

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:192 SpawnActionTemplate.isShareable
    fn is_shareable(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:202 SpawnActionTemplate.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        self.common_tools.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:212 SpawnActionTemplate.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        self.all_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:217 SpawnActionTemplate.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:222 SpawnActionTemplate.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        self.all_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:227 SpawnActionTemplate.getInputFilesForExtraAction
    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        Some(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:233 SpawnActionTemplate.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:238 SpawnActionTemplate.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        self.spawn_action_builder
            .build_for_action_template(self.get_owner().clone())
            .get_client_environment_variables()
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:99 ActionTemplate.getPrimaryInput
    fn get_primary_input(&self) -> Option<Artifact> {
        Some(self.get_input_tree_artifacts()[0].clone())
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:109 ActionTemplate.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        IndexMap::new()
    }

    // Java: com/google/devtools/build/lib/actions/ActionTemplate.java:115 ActionTemplate.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        None
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:243 SpawnActionTemplate.prettyPrint
    fn pretty_print(&self) -> String {
        format!(
            "SpawnActionTemplate with output TreeArtifact {}",
            self.output_tree_artifact.pretty_print()
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:248 SpawnActionTemplate.describe
    fn describe(&self) -> String {
        format!(
            "Executing {} action on all files in {}",
            self.mnemonic,
            self.input_tree_artifact.pretty_print()
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:253 SpawnActionTemplate.toString
    fn to_string_java(&self) -> String {
        self.pretty_print()
    }
}

/// Builder class to construct [`SpawnActionTemplate`] instances.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:258 SpawnActionTemplate.Builder
pub struct SpawnActionTemplateBuilder {
    action_template_mnemonic: String,
    output_path_mapper: Option<OutputPathMapper>,
    command_line_template: Option<Arc<CustomCommandLine>>,
    executable: Option<PathFragment>,
    input_tree_artifact: Artifact,
    output_tree_artifact: Artifact,
    inputs_builder: NestedSetBuilder<Artifact>,
    tools_builder: NestedSetBuilder<Artifact>,
    spawn_action_builder: SpawnActionBuilder,
}

impl SpawnActionTemplateBuilder {
    /// Creates a [`SpawnActionTemplate`] builder.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:278 SpawnActionTemplate.Builder.<init>
    pub fn new(input_tree_artifact: Artifact, output_tree_artifact: Artifact) -> Self {
        blaze_util::check_state!(
            input_tree_artifact.is_tree_artifact() && output_tree_artifact.is_tree_artifact(),
            "Either {} or {} is not a TreeArtifact",
            input_tree_artifact,
            output_tree_artifact
        );
        SpawnActionTemplateBuilder {
            action_template_mnemonic: "Unknown".to_owned(),
            output_path_mapper: None,
            command_line_template: None,
            executable: None,
            input_tree_artifact,
            output_tree_artifact,
            inputs_builder: NestedSetBuilder::stable_order(),
            tools_builder: NestedSetBuilder::stable_order(),
            spawn_action_builder: SpawnActionBuilder::new(),
        }
    }

    /// Sets the mnemonics for both the [`SpawnActionTemplate`] and the expanded
    /// [`SpawnAction`]s.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:291 SpawnActionTemplate.Builder.setMnemonics
    pub fn set_mnemonics(
        &mut self,
        action_template_mnemonic: &str,
        expanded_action_mnemonic: &str,
    ) -> &mut Self {
        self.action_template_mnemonic = action_template_mnemonic.to_owned();
        self.spawn_action_builder
            .set_mnemonic(expanded_action_mnemonic);
        self
    }

    /// Adds common tool artifacts. All common tool artifacts will be added as tool artifacts
    /// for expanded actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:302 SpawnActionTemplate.Builder.addCommonTools
    pub fn add_common_tools(&mut self, artifacts: impl IntoIterator<Item = Artifact>) -> &mut Self {
        let artifacts: Vec<Artifact> = artifacts.into_iter().collect();
        self.tools_builder.add_all(artifacts.iter().cloned());
        self.spawn_action_builder.add_tools(artifacts);
        self
    }

    /// Adds common tool artifacts. All common tool artifacts will be added as input tool
    /// artifacts for expanded actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:313 SpawnActionTemplate.Builder.addCommonTool
    pub fn add_common_tool(&mut self, tool: &FilesToRunProvider) -> &mut Self {
        self.tools_builder.add_transitive(tool.get_files_to_run());
        // spawnActionBuilder.addTool(tool) = addTransitiveTools(tool.getFilesToRun())
        self.spawn_action_builder
            .add_transitive_tools(tool.get_files_to_run());
        self
    }

    /// Adds common input artifacts. All common input artifacts will be added as input artifacts
    /// for expanded actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:324 SpawnActionTemplate.Builder.addCommonInputs
    pub fn add_common_inputs(
        &mut self,
        artifacts: impl IntoIterator<Item = Artifact>,
    ) -> &mut Self {
        let artifacts: Vec<Artifact> = artifacts.into_iter().collect();
        self.inputs_builder.add_all(artifacts.iter().cloned());
        self.spawn_action_builder.add_inputs(artifacts);
        self
    }

    /// Sets the map of environment variables for expanded actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:334 SpawnActionTemplate.Builder.setEnvironment
    pub fn set_environment(&mut self, environment: IndexMap<String, String>) -> &mut Self {
        self.spawn_action_builder.set_environment(environment);
        self
    }

    /// Sets the map of execution info for expanded actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:341 SpawnActionTemplate.Builder.setExecutionInfo
    pub fn set_execution_info(&mut self, execution_info: IndexMap<String, String>) -> &mut Self {
        self.spawn_action_builder.set_execution_info(execution_info);
        self
    }

    /// Sets the executable used by expanded actions as a configured target. Automatically adds
    /// the files to run to the tools and uses the executable of the target as the executable.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:354 SpawnActionTemplate.Builder.setExecutable
    pub fn set_executable_files_to_run(
        &mut self,
        executable_provider: &FilesToRunProvider,
    ) -> &mut Self {
        let executable = executable_provider
            .get_executable()
            .unwrap_or_else(|| {
                panic!("IllegalArgumentException: The target does not have an executable")
            })
            .clone();
        self.spawn_action_builder.set_executable_files_to_run(
            Some(executable.clone()),
            executable_provider.get_files_to_run(),
        );
        self.add_common_tool(executable_provider);
        self.executable = Some(executable.get_exec_path().clone());
        self
    }

    /// Sets the executable path used by expanded actions. The path is interpreted relative to
    /// the execution root, unless it's a bare file name.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:375 SpawnActionTemplate.Builder.setExecutable
    pub fn set_executable_path(&mut self, executable: PathFragment) -> &mut Self {
        self.spawn_action_builder.set_executable_path(&executable);
        self.executable = Some(executable);
        self
    }

    /// Sets the executable artifact used by expanded actions. The path is interpreted relative
    /// to the execution root.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:389 SpawnActionTemplate.Builder.setExecutable
    pub fn set_executable(&mut self, artifact: Artifact) -> &mut Self {
        self.spawn_action_builder.set_executable(artifact.clone());
        self.add_common_tools([artifact.clone()]);
        self.executable = Some(artifact.get_exec_path().clone());
        self
    }

    /// Sets the command line template used to expand actions.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:398 SpawnActionTemplate.Builder.setCommandLineTemplate
    pub fn set_command_line_template(
        &mut self,
        command_line_template: CustomCommandLine,
    ) -> &mut Self {
        self.command_line_template = Some(Arc::new(command_line_template));
        self
    }

    /// Sets the [`OutputPathMapper`] used to get the parent-relative paths of output
    /// `TreeFileArtifact`s.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:408 SpawnActionTemplate.Builder.setOutputPathMapper
    pub fn set_output_path_mapper(&mut self, output_path_mapper: OutputPathMapper) -> &mut Self {
        self.output_path_mapper = Some(output_path_mapper);
        self
    }

    /// Builds and returns the [`SpawnActionTemplate`] using the accumulated builder
    /// information.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnActionTemplate.java:418 SpawnActionTemplate.Builder.build
    pub fn build(&mut self, action_owner: ActionOwner) -> SpawnActionTemplate {
        // checkNotNull(executable)
        if self.executable.is_none() {
            panic!("NullPointerException: executable");
        }
        SpawnActionTemplate::new(
            action_owner,
            self.input_tree_artifact.clone(),
            self.output_tree_artifact.clone(),
            self.inputs_builder.build(),
            self.tools_builder.build(),
            self.output_path_mapper
                .clone()
                .expect("NullPointerException: outputPathMapper"),
            self.command_line_template
                .clone()
                .expect("NullPointerException: commandLineTemplate"),
            self.action_template_mnemonic.clone(),
            SpawnActionBuilder::from_builder(&self.spawn_action_builder),
        )
    }
}
