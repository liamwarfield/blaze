//! Port of `lib/rules/cpp/CppCompileActionTemplate.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, ActionRef, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::actions::platform_info::PlatformInfoRef;
use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_cc_features::actions::NOOP as CC_NOOP;
use blaze_cc_features::artifact_category::ArtifactCategory;
use blaze_cc_features::cc_toolchain_features::CcToolchainFeatures;
use blaze_cc_features::cc_toolchain_variables::CcToolchainVariables;
use blaze_cc_features::compile_build_variables::CompileBuildVariables;
use blaze_cc_features::cpp_action_names::CppActionNames;
use blaze_collect::command_line_item::CommandLineExpansionException;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::handler::EventHandler;
use blaze_exec::actions::{ActionExecutionException, ActionTemplate, ActionTemplateError};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use super::cc_artifact::to_cc_artifact;
use super::cpp_compile_action::{
    BuildAndVerifyError, CppCompileAction, CppCompileActionBuilder, build_command_line,
    compute_command_line_key, compute_key_static,
};
use super::cpp_file_types;

/// An `ActionTemplate` that expands into `CppCompileAction`s at execution time.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:51 CppCompileActionTemplate
pub struct CppCompileActionTemplate {
    /// Owner, `getInputs()` (the source tree, then `allInputs`) and `getOutputs()`.
    base: AbstractAction,
    cpp_compile_action_builder: CppCompileActionBuilder,
    source_tree_artifact: Artifact,
    output_tree_artifact: Artifact,
    dotd_tree_artifact: Option<Artifact>,
    diagnostics_tree_artifact: Option<Artifact>,
    lto_index_tree_artifact: Option<Artifact>,
    /// `toolchain.getFeatures()` (Java reads it when the template is expanded; the getter's
    /// `EvalException` message is kept and raised there).
    toolchain_features: Result<Option<Arc<CcToolchainFeatures>>, String>,
    categories: Vec<ArtifactCategory>,
    mandatory_inputs: NestedSet<Artifact>,
}

impl std::fmt::Debug for CppCompileActionTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

// LINT.IfChange(cc_and_objc_file_types)
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:115 CppCompileActionTemplate.CC_AND_OBJC_FILE_TYPES
fn cc_and_objc_file_types_matches(path: &str) -> bool {
    cpp_file_types::CPP_SOURCE.matches(path)
        || cpp_file_types::CPP_HEADER.matches(path)
        || cpp_file_types::OBJC_SOURCE.matches(path)
        || cpp_file_types::OBJCPP_SOURCE.matches(path)
        || cpp_file_types::C_SOURCE.matches(path)
        || cpp_file_types::ASSEMBLER.matches(path)
        || cpp_file_types::ASSEMBLER_WITH_C_PREPROCESSOR.matches(path)
}
// LINT.ThenChange(@rules_cc//cc/private/compile/compile.bzl:cc_and_objc_file_types)

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:232 CppFileTypes.LTO_INDEXING_OBJECT_FILE
const LTO_INDEXING_OBJECT_FILE_EXTENSION: &str = ".indexing.o";

impl CppCompileActionTemplate {
    /// Creates a CppCompileActionTemplate.
    ///
    /// - `source_tree_artifact`: the TreeArtifact that contains source files to compile.
    /// - `output_tree_artifact`: the TreeArtifact that contains compilation outputs.
    /// - `dotd_tree_artifact`: the TreeArtifact that contains dotd files.
    /// - `diagnostics_tree_artifact`: the TreeArtifact that contains serialized diagnostics files.
    /// - `lto_index_tree_artifact`: the TreeArtifact that contains lto index files (minimized
    ///   bitcode).
    /// - `cpp_compile_action_builder`: An almost completely configured `CppCompileActionBuilder`
    ///   without the input and output files set. It is used as a template to instantiate
    ///   expanded `CppCompileAction`s.
    /// - `toolchain_features`: `getFeatures()` of the CcToolchainProvider representing the c++
    ///   toolchain for this action.
    /// - `categories`: A list of `ArtifactCategory` used to calculate output file name from a
    ///   source file name.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:83 CppCompileActionTemplate.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_tree_artifact: Artifact,
        output_tree_artifact: Artifact,
        dotd_tree_artifact: Option<Artifact>,
        diagnostics_tree_artifact: Option<Artifact>,
        lto_index_tree_artifact: Option<Artifact>,
        mut cpp_compile_action_builder: CppCompileActionBuilder,
        toolchain_features: Result<Option<Arc<CcToolchainFeatures>>, String>,
        categories: Vec<ArtifactCategory>,
    ) -> Result<CppCompileActionTemplate, BuildAndVerifyError> {
        let action_owner = cpp_compile_action_builder.get_owner().clone();
        let mandatory_inputs = cpp_compile_action_builder.build_mandatory_inputs_for_template()?;
        let mut all_inputs_builder = NestedSetBuilder::from_nested_set(&mandatory_inputs);
        all_inputs_builder.add_transitive(cpp_compile_action_builder.get_inputs_for_invalidation());
        let all_inputs = all_inputs_builder.build();

        // getInputs(): stableOrder().add(sourceTreeArtifact).addTransitive(allInputs).
        let mut inputs = NestedSetBuilder::stable_order();
        inputs
            .add(source_tree_artifact.clone())
            .add_transitive(&all_inputs);
        // getOutputs(): outputTreeArtifact, dotdTreeArtifact, ltoIndexTreeArtifact (the
        // diagnostics tree is not an output in Java).
        let mut outputs = vec![output_tree_artifact.clone()];
        if let Some(dotd) = &dotd_tree_artifact {
            outputs.push(dotd.clone());
        }
        if let Some(lto) = &lto_index_tree_artifact {
            outputs.push(lto.clone());
        }
        Ok(CppCompileActionTemplate {
            base: AbstractAction::new(action_owner, inputs.build(), outputs),
            cpp_compile_action_builder,
            source_tree_artifact,
            output_tree_artifact,
            dotd_tree_artifact,
            diagnostics_tree_artifact,
            lto_index_tree_artifact,
            toolchain_features,
            categories,
            mandatory_inputs,
        })
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:265 CppCompileActionTemplate.shouldCompileHeaders
    fn should_compile_headers(&self) -> bool {
        self.cpp_compile_action_builder.should_compile_headers()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:269 CppCompileActionTemplate.createAction
    fn create_action(
        &self,
        source_tree_file_artifact: &Artifact,
        output_tree_file_artifact: &Artifact,
        dotd_file_artifact: Option<Artifact>,
        diagnostics_file_artifact: Option<Artifact>,
        lto_index_file_artifact: Option<Artifact>,
        private_headers: NestedSet<Artifact>,
    ) -> Result<CppCompileAction, ActionExecutionException> {
        let mut builder = CppCompileActionBuilder::copy_of(&self.cpp_compile_action_builder);
        builder
            .set_additional_prunable_headers(private_headers)
            // TreeFileArtifact overload of setSourceFile.
            .set_source_file_tree_file(source_tree_file_artifact.clone())
            .set_outputs(
                Some(output_tree_file_artifact.clone()),
                dotd_file_artifact.clone(),
                diagnostics_file_artifact.clone(),
            )
            .set_lto_indexing_file(lto_index_file_artifact.clone());

        let mut build_variables = CcToolchainVariables::builder_with_parent(Some(
            self.cpp_compile_action_builder.get_variables().clone(),
        ));
        build_variables.override_variable(
            CompileBuildVariables::SOURCE_FILE.get_variable_name(),
            to_cc_artifact(source_tree_file_artifact),
        );
        build_variables.override_variable(
            CompileBuildVariables::OUTPUT_FILE.get_variable_name(),
            to_cc_artifact(output_tree_file_artifact),
        );
        if let Some(dotd_file_artifact) = &dotd_file_artifact {
            build_variables.override_variable(
                CompileBuildVariables::DEPENDENCY_FILE.get_variable_name(),
                to_cc_artifact(dotd_file_artifact),
            );
        }
        if let Some(diagnostics_file_artifact) = &diagnostics_file_artifact {
            build_variables.override_variable(
                CompileBuildVariables::SERIALIZED_DIAGNOSTICS_FILE.get_variable_name(),
                to_cc_artifact(diagnostics_file_artifact),
            );
        }
        if let Some(lto_index_file_artifact) = &lto_index_file_artifact {
            build_variables.override_variable(
                CompileBuildVariables::LTO_INDEXING_BITCODE_FILE.get_variable_name(),
                to_cc_artifact(lto_index_file_artifact),
            );
        }
        builder.set_variables(build_variables.build());
        match builder.build_and_verify() {
            Ok(action) => Ok(action),
            Err(BuildAndVerifyError::UnconfiguredActionConfig(e)) => {
                Err(self.throw_action_execution_exception(&e.message))
            }
            Err(BuildAndVerifyError::Eval(e)) => {
                Err(self.throw_action_execution_exception(&e.to_string()))
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:313 CppCompileActionTemplate.outputTreeFileArtifactName
    fn output_tree_file_artifact_name(
        &self,
        input_tree_file_artifact: &Artifact,
    ) -> Result<String, String> {
        let parent_relative_path = input_tree_file_artifact
            .get_parent_relative_path()
            .unwrap_or_else(|| panic!("NullPointerException: {input_tree_file_artifact}"));
        let mut output_name = remove_extension(parent_relative_path.get_path_string()).to_owned();
        for category in &self.categories {
            let features = self
                .toolchain_features
                .clone()?
                .unwrap_or_else(|| panic!("NullPointerException: toolchain features"));
            output_name = features.get_artifact_name_for_category(*category, &output_name);
        }
        Ok(output_name)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:323 CppCompileActionTemplate.throwActionExecutionException
    fn throw_action_execution_exception(&self, cause_message: &str) -> ActionExecutionException {
        // new ActionExecutionException(cause, this, false, makeDetailedExitCode(cause.getMessage()))
        ActionExecutionException::with_cause(
            cause_message,
            &cause_message,
            Some(self),
            /* catastrophe= */ false,
            Some(make_detailed_exit_code(cause_message)),
        )
    }
}

/// `FileSystemUtils.removeExtension(String)`: removes the shortest suffix beginning with `.` from
/// the last path segment; unchanged when it has none.
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

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:395 CppCompileActionTemplate.makeDetailedExitCode
fn make_detailed_exit_code(message: &str) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::Execution(failure_details::Execution {
            code: failure_details::execution::Code::PersistentActionOutputDirectoryCreationFailure
                as i32,
        })),
    }
}

impl ActionTemplate for CppCompileActionTemplate {
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:126 CppCompileActionTemplate.generateActionsForInputArtifacts
    fn generate_actions_for_input_artifacts(
        &self,
        input_tree_file_artifacts: Vec<Artifact>,
        artifact_owner: ActionLookupKeyRef,
        _event_handler: &dyn EventHandler,
    ) -> Result<Vec<ActionRef>, ActionTemplateError> {
        let mut expanded_actions: Vec<ActionRef> = Vec::new();
        let mut sources_builder: Vec<Artifact> = Vec::new();
        let mut private_headers_builder = NestedSetBuilder::stable_order();
        for input_tree_file_artifact in &input_tree_file_artifacts {
            let is_header =
                cpp_file_types::CPP_HEADER.matches_path(input_tree_file_artifact.get_exec_path());
            let is_textual_include = cpp_file_types::CPP_TEXTUAL_INCLUDE
                .matches_path(input_tree_file_artifact.get_exec_path());
            let is_source =
                cc_and_objc_file_types_matches(input_tree_file_artifact.get_exec_path_string())
                    && !is_header;
            if is_header {
                private_headers_builder.add(input_tree_file_artifact.clone());
            }
            if is_source || (is_header && self.should_compile_headers() && !is_textual_include) {
                sources_builder.push(input_tree_file_artifact.clone());
            } else if !is_header {
                let message = format!(
                    "Artifact '{}' expanded from the directory artifact '{}' is neither header \
                     nor source file.",
                    input_tree_file_artifact.get_exec_path_string(),
                    self.source_tree_artifact.get_exec_path_string()
                );
                return Err(ActionExecutionException::new(
                    message.clone(),
                    Some(self),
                    /* catastrophe= */ false,
                    Some(make_detailed_exit_code(&message)),
                )
                .into());
            }
        }
        let sources = sources_builder;
        let private_headers = private_headers_builder.build();
        for input_tree_file_artifact in &sources {
            let output_name = self
                .output_tree_file_artifact_name(input_tree_file_artifact)
                .map_err(|e| self.throw_action_execution_exception(&e))?;
            let output_tree_file_artifact = Artifact::create_template_expansion_output(
                &self.output_tree_artifact,
                PathFragment::create(&output_name),
                artifact_owner.clone(),
            );
            let mut dotd_file_artifact = None;
            if let Some(dotd_tree_artifact) = &self.dotd_tree_artifact
                && self
                    .cpp_compile_action_builder
                    .use_dotd_file(input_tree_file_artifact)
            {
                dotd_file_artifact = Some(Artifact::create_template_expansion_output(
                    dotd_tree_artifact,
                    PathFragment::create(&format!("{output_name}.d")),
                    artifact_owner.clone(),
                ));
            }
            let mut diagnostics_file_artifact = None;
            if let Some(diagnostics_tree_artifact) = &self.diagnostics_tree_artifact {
                diagnostics_file_artifact = Some(Artifact::create_template_expansion_output(
                    diagnostics_tree_artifact,
                    PathFragment::create(&format!("{output_name}.dia")),
                    artifact_owner.clone(),
                ));
            }
            let mut lto_index_file_artifact = None;
            if let Some(lto_index_tree_artifact) = &self.lto_index_tree_artifact {
                let output_file_path_fragment = PathFragment::create(&output_name);
                // Iterables.getOnlyElement(CppFileTypes.LTO_INDEXING_OBJECT_FILE.getExtensions())
                let thinltofile = replace_extension(
                    &output_file_path_fragment,
                    LTO_INDEXING_OBJECT_FILE_EXTENSION,
                );
                lto_index_file_artifact = Some(Artifact::create_template_expansion_output(
                    lto_index_tree_artifact,
                    thinltofile,
                    artifact_owner.clone(),
                ));
            }
            expanded_actions.push(Arc::new(self.create_action(
                input_tree_file_artifact,
                &output_tree_file_artifact,
                dotd_file_artifact,
                diagnostics_file_artifact,
                lto_index_file_artifact,
                private_headers.clone(),
            )?));
        }
        Ok(expanded_actions)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:328 CppCompileActionTemplate.getInputTreeArtifacts
    fn get_input_tree_artifacts(&self) -> Vec<Artifact> {
        vec![self.source_tree_artifact.clone()]
    }
}

impl Action for CppCompileActionTemplate {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:343 CppCompileActionTemplate.getMnemonic
    fn get_mnemonic(&self) -> String {
        "CppCompileActionTemplate".to_owned()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:222 CppCompileActionTemplate.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        let builder = &self.cpp_compile_action_builder;
        let command_line = build_command_line(
            builder.get_copts_filter().clone(),
            CppActionNames::CPP_COMPILE,
            builder.get_feature_configuration().clone(),
            builder.get_variables().clone(),
        );
        let expansion = |message: String| {
            ComputeKeyError::CommandLineExpansion(CommandLineExpansionException::new(message))
        };
        let environment_variables = command_line
            .get_environment(&CC_NOOP)
            .map_err(|e| expansion(e.message))?;
        let compiler_options = command_line
            .get_compiler_options(/* overwritten_variables= */ None, &CC_NOOP)
            .map_err(|e| expansion(e.message))?;
        // catch (EvalException e) { throw new CommandLineExpansionException(e.getMessage()); }
        let built_in_include_directories = builder
            .get_builtin_include_directories()
            .map_err(expansion)?;
        compute_key_static(
            action_key_context,
            fp,
            &builder.get_action_environment(),
            &environment_variables,
            builder.get_execution_info(),
            &compute_command_line_key(&compiler_options),
            builder
                .get_cc_compilation_context()
                .get_declared_include_srcs(),
            &self.mandatory_inputs,
            &self.mandatory_inputs,
            builder.get_prunable_headers(),
            &built_in_include_directories,
            builder.get_inputs_for_invalidation(),
            &self.get_mnemonic(),
            // This method is not called during actual execution (action templates are always
            // expanded into individual actions that then have their action key computed), so
            // path mapping is supported and fingerprinted correctly even with this set to OFF.
            OutputPathsMode::OFF,
        )
    }

    /// An `ActionTemplate` is an `ActionAnalysisMetadata`, not an `ActionExecutionMetadata`.
    fn is_action_execution_metadata(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:338 CppCompileActionTemplate.isShareable
    fn is_shareable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:348 CppCompileActionTemplate.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        let mut b = NestedSetBuilder::compile_order();
        b.add(self.source_tree_artifact.clone())
            .add_transitive(&self.mandatory_inputs);
        b.build()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:361 CppCompileActionTemplate.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:366 CppCompileActionTemplate.getTools
    fn get_tools(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:384 CppCompileActionTemplate.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:401 CppCompileActionTemplate.getClientEnvironmentVariables
    fn get_client_environment_variables(&self) -> Vec<String> {
        Vec::new()
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

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:406 CppCompileActionTemplate.prettyPrint
    fn pretty_print(&self) -> String {
        format!(
            "CppCompileActionTemplate compiling {}",
            self.source_tree_artifact.get_exec_path_string()
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:411 CppCompileActionTemplate.describe
    fn describe(&self) -> String {
        format!(
            "Compiling all C++ files in {}",
            self.source_tree_artifact.pretty_print()
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionTemplate.java:416 CppCompileActionTemplate.toString
    fn to_string_java(&self) -> String {
        self.pretty_print()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_extension_matches_file_system_utils() {
        assert_eq!(remove_extension("foo.cc"), "foo");
        assert_eq!(remove_extension("dir/foo.pic.cc"), "dir/foo.pic");
        assert_eq!(remove_extension("dir.d/foo"), "dir.d/foo");
        assert_eq!(remove_extension("foo"), "foo");
    }

    #[test]
    fn replace_extension_replaces_last_extension_of_base_name() {
        assert_eq!(
            replace_extension(&PathFragment::create("a/b.pic.o"), ".indexing.o"),
            PathFragment::create("a/b.pic.indexing.o")
        );
    }
}
