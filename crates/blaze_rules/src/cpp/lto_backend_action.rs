//! Ports of `rules/cpp/LtoBackendAction.java` (the ThinLTO backend compile), the Starlark half of
//! `rules/cpp/LtoBackendArtifacts.java` (`createLtoBackendActionForStarlark`,
//! `getLtoBackendCommandLine`) and `rules/cpp/BitcodeFiles.java`.
//!
//! PORT: Java's `LtoBackendAction extends SpawnAction`; here it wraps a [`SpawnAction`] made with
//! the same arguments and delegates to it everything Java inherits, overriding what Java
//! overrides (the key, input discovery from the imports file, the mandatory and allowed derived
//! inputs). The spawn runs the wrapped action's `execute`, which reads the inputs of the shared
//! `AbstractAction` (updated by discovery).

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, OnceLock};

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::command_line::CommandLine;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::fingerprint_util::FingerprintPathExt;
use blaze_actions::actions::path_mapper::{OutputPathsMode, PathMapper};
use blaze_actions::analysis::actions::spawn_action::{ResourceSet, SpawnAction};
use blaze_cc_features::cc_toolchain_features::FeatureConfiguration;
use blaze_cc_features::cc_toolchain_variables::CcToolchainVariablesRef;
use blaze_cc_features::cpp_action_names::CppActionNames;
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use blaze_exec::analysis::actions::get_input_path;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::{Fingerprint, check_state};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::cc_artifact::{CcInputMetadataProvider, CcPathMapper};

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:52 LtoBackendAction.GUID
const GUID: &str = "72ce1eca-4625-4e24-a0d8-bb91bb8b0e0e";
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:53 LtoBackendAction.MNEMONIC
pub const MNEMONIC: &str = "CcLtoBackendCompile";

/// Wrapper for a set of bitcode files, with a cached fingerprint and exec-path map.
// Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:27 BitcodeFiles
pub struct BitcodeFiles {
    files: NestedSet<Artifact>,
    fingerprint: OnceLock<Vec<u8>>,
    files_artifact_path_map: OnceLock<HashMap<PathFragment, Artifact>>,
}

impl BitcodeFiles {
    // Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:39 BitcodeFiles.<init>
    pub fn new(files: NestedSet<Artifact>) -> BitcodeFiles {
        BitcodeFiles {
            files,
            fingerprint: OnceLock::new(),
            files_artifact_path_map: OnceLock::new(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:43 BitcodeFiles.getFiles
    pub fn get_files(&self) -> &NestedSet<Artifact> {
        &self.files
    }

    /// The files by exec path (Java caches it weakly; here for the value's lifetime).
    // Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:48 BitcodeFiles.getFilesArtifactPathMap
    pub fn get_files_artifact_path_map(&self) -> &HashMap<PathFragment, Artifact> {
        self.files_artifact_path_map.get_or_init(|| {
            let files_list = self.get_files().to_list();
            let mut result = HashMap::with_capacity(files_list.len());
            for file in files_list.iter() {
                result.insert(file.get_exec_path().clone(), file.clone());
            }
            result
        })
    }

    // Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:77 BitcodeFiles.addToFingerprint
    pub fn add_to_fingerprint(&self, fp: &mut Fingerprint) {
        let fingerprint = self.fingerprint.get_or_init(|| self.compute_fingerprint());
        fp.add_bytes(fingerprint);
    }

    // Java: com/google/devtools/build/lib/rules/cpp/BitcodeFiles.java:88 BitcodeFiles.computeFingerprint
    fn compute_fingerprint(&self) -> Vec<u8> {
        let mut fp = Fingerprint::new();
        for path in self.files.to_list().iter() {
            fp.add_path_fragment(path.get_exec_path());
        }
        fp.digest_and_reset()
    }
}

/// The LTO backend command line: the feature configuration's `lto-backend` command line, then
/// `-fPIC` for a PIC compile (after the rest so that it cannot be overridden, consistent with
/// the ordering in the CppCompileAction's compiler options).
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:133 LtoBackendArtifacts.getLtoBackendCommandLine
struct LtoBackendCommandLine {
    feature_configuration: Arc<FeatureConfiguration>,
    build_variables: CcToolchainVariablesRef,
    use_pic: bool,
}

impl fmt::Debug for LtoBackendCommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LtoBackendCommandLine")
    }
}

impl CommandLine for LtoBackendCommandLine {
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.arguments_with(None, &blaze_actions::actions::path_mapper::NOOP)
    }

    fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let imp = input_metadata_provider.map(CcInputMetadataProvider);
        let imp_ref: Option<&dyn blaze_cc_features::actions::InputMetadataProvider> = imp
            .as_ref()
            .map(|p| p as &dyn blaze_cc_features::actions::InputMetadataProvider);
        let mut args = self
            .feature_configuration
            .get_command_line_with(
                CppActionNames::LTO_BACKEND,
                &self.build_variables,
                imp_ref,
                &CcPathMapper(path_mapper),
            )
            // catch (ExpansionException e) { throw new CommandLineExpansionException(...) }
            .map_err(|e| {
                MapFnError::CommandLineExpansion(CommandLineExpansionException::new(e.message))
            })?;
        if self.use_pic {
            args.push("-fPIC".to_owned());
        }
        Ok(args)
    }
}

/// Action for LTO that reads the imports file to determine its inputs.
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:51 LtoBackendAction
pub struct LtoBackendAction {
    spawn: SpawnAction,
    mandatory_inputs: NestedSet<Artifact>,
    bitcode_files: Option<Arc<BitcodeFiles>>,
    imports: Option<Artifact>,
    inputs_discovered: Mutex<bool>,
}

impl fmt::Debug for LtoBackendAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.spawn, f)
    }
}

impl LtoBackendAction {
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:60 LtoBackendAction.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        all_bitcode_files: Option<Arc<BitcodeFiles>>,
        imports_file: Option<Artifact>,
        outputs: Vec<Artifact>,
        argv: CommandLines,
        env: Arc<ActionEnvironment>,
        execution_info: IndexMap<String, String>,
    ) -> LtoBackendAction {
        let spawn = SpawnAction::new(
            owner,
            /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            inputs.clone(),
            outputs,
            ResourceSet::default_resource_set(),
            argv,
            env,
            &execution_info,
            Some("LTO Backend Compile %{output}".to_owned()),
            MNEMONIC,
            OutputPathsMode::OFF,
        )
        .with_java_class("com.google.devtools.build.lib.rules.cpp.LtoBackendAction");
        check_state!(
            all_bitcode_files.is_none() == imports_file.is_none(),
            "Either both or neither bitcodeFiles and imports files should be null"
        );
        LtoBackendAction {
            spawn,
            mandatory_inputs: inputs,
            bitcode_files: all_bitcode_files,
            imports: imports_file,
            inputs_discovered: Mutex::new(false),
        }
    }

    /// `LtoBackendAction.create`: the execution info is the configuration's
    /// `modifiedExecutionInfo(ImmutableMap.of(), MNEMONIC)`, empty without a configuration.
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:305 LtoBackendAction.create
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        action_owner: ActionOwner,
        configuration: Option<
            &dyn blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration,
        >,
        inputs: NestedSet<Artifact>,
        all_bitcode_files: Option<Arc<BitcodeFiles>>,
        imports_file: Option<Artifact>,
        outputs: Vec<Artifact>,
        argv: CommandLines,
        env: Arc<ActionEnvironment>,
    ) -> LtoBackendAction {
        let execution_info = match configuration {
            None => IndexMap::new(),
            Some(configuration) => {
                configuration.modified_execution_info(&IndexMap::new(), MNEMONIC)
            }
        };
        LtoBackendAction::new(
            action_owner,
            inputs,
            all_bitcode_files,
            imports_file,
            outputs,
            argv,
            env,
            execution_info,
        )
    }

    fn bitcode_files(&self) -> &BitcodeFiles {
        self.bitcode_files
            .as_deref()
            .expect("NullPointerException: bitcodeFiles")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:134 LtoBackendAction.getArtifactOrTreeArtifact
    fn get_artifact_or_tree_artifact(
        path: &PathFragment,
        path_to_artifact: &HashMap<PathFragment, Artifact>,
    ) -> Option<Artifact> {
        let mut current_path = path.clone();
        while !current_path.is_empty() {
            if let Some(artifact) = path_to_artifact.get(&current_path) {
                return Some(artifact.clone());
            }
            current_path = current_path
                .get_parent_directory()
                .unwrap_or_else(PathFragment::empty_fragment);
        }
        None
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:147 LtoBackendAction.computeBitcodeInputs
    fn compute_bitcode_inputs(
        &self,
        input_paths: &[PathFragment],
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<NestedSet<Artifact>, ActionExecutionException> {
        let mut bitcode_inputs = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        let exec_path_to_artifact = self.bitcode_files().get_files_artifact_path_map();
        let mut missing_inputs: Vec<PathFragment> = Vec::new();
        for input_path in input_paths {
            match Self::get_artifact_or_tree_artifact(input_path, exec_path_to_artifact) {
                Some(artifact) => {
                    bitcode_inputs.add(artifact);
                }
                // One of the inputs is not present. We add it to missingInputs and will fail.
                None => missing_inputs.push(input_path.clone()),
            }
        }
        if !missing_inputs.is_empty() {
            // Limit the reported count to protect against a large error message.
            let mut missing: Vec<String> = missing_inputs
                .iter()
                .map(|p| p.get_path_string().to_owned())
                .collect();
            missing.sort();
            missing.truncate(10);
            let message = format!(
                "error computing inputs from imports file: {}, missing bitcode files (first 10): \
                 {}",
                self.imports_input_path(action_execution_context)
                    .get_path_string(),
                missing.join(", ")
            );
            let code = create_failure_detail(
                &message,
                failure_details::lto_action::Code::MissingBitcodeFiles,
            );
            return Err(ActionExecutionException::new(
                message,
                Some(self),
                false,
                Some(code),
            ));
        }
        Ok(bitcode_inputs.build())
    }

    fn imports_input_path(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> blaze_vfs::Path {
        get_input_path(
            action_execution_context,
            self.imports.as_ref().expect("an imports file"),
        )
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:291 LtoBackendAction.createDetailedExitCode
fn create_failure_detail(
    message: &str,
    detailed_code: failure_details::lto_action::Code,
) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::LtoAction(failure_details::LtoAction {
            code: detailed_code as i32,
        })),
    }
}

impl Action for LtoBackendAction {
    fn java_class_name(&self) -> Option<&'static str> {
        self.spawn.java_class_name()
    }

    fn abstract_action(&self) -> &AbstractAction {
        self.spawn.abstract_action()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_mnemonic(&self) -> String {
        self.spawn.get_mnemonic()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:271 LtoBackendAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        match self.spawn.get_arguments() {
            Ok(arguments) => {
                fp.add_strings(arguments.iter());
            }
            Err(e) => {
                panic!("AssertionError: LtoBackendAction command line expansion cannot fail: {e:?}")
            }
        }
        fp.add_string(&self.get_mnemonic());
        for input in self.mandatory_inputs.to_list().iter() {
            fp.add_path_fragment(input.get_exec_path());
        }
        if let Some(imports) = &self.imports {
            self.bitcode_files().add_to_fingerprint(fp);
            fp.add_path_fragment(imports.get_exec_path());
        }
        self.get_environment().add_to(fp);
        let execution_info = self.get_execution_info();
        fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        Ok(())
    }

    fn describe_key(&self) -> Option<String> {
        self.spawn.describe_key()
    }

    fn get_tools(&self) -> NestedSet<Artifact> {
        self.spawn.get_tools()
    }

    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.spawn.get_environment()
    }

    fn is_volatile(&self) -> bool {
        self.spawn.is_volatile()
    }

    fn get_raw_progress_message(&self) -> Option<String> {
        self.spawn.get_raw_progress_message()
    }

    fn get_execution_info(&self) -> IndexMap<String, String> {
        self.spawn.get_execution_info()
    }

    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        self.spawn.get_scheduling_dependencies()
    }

    fn is_shareable(&self) -> bool {
        self.spawn.is_shareable()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:116 LtoBackendAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        self.imports.is_some()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:121 LtoBackendAction.inputsDiscovered
    fn inputs_discovered(&self) -> bool {
        *self.inputs_discovered.lock()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:126 LtoBackendAction.setInputsDiscovered
    fn set_inputs_discovered(&self, inputs_discovered: bool) {
        *self.inputs_discovered.lock() = inputs_discovered;
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:250 LtoBackendAction.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:261 LtoBackendAction.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:266 LtoBackendAction.getAllowedDerivedInputs
    fn get_allowed_derived_inputs(&self) -> NestedSet<Artifact> {
        self.bitcode_files().get_files().clone()
    }

    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
        path_mapper: &dyn PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        self.spawn
            .get_effective_environment(client_env, path_mapper)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        self.spawn.get_input_files_for_extra_action()
    }

    fn get_extra_action_info(
        &self,
        action_key_context: &ActionKeyContext,
    ) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
        self.spawn.get_extra_action_info(action_key_context)
    }

    fn get_starlark_args(
        &self,
    ) -> Option<Vec<blaze_actions::analysis::starlark::args::RegisteredArgs>> {
        self.spawn.get_starlark_args()
    }

    fn get_starlark_argv(
        &self,
    ) -> Result<Option<Vec<String>>, blaze_actions::actions::action::ActionApiError> {
        self.spawn.get_starlark_argv()
    }
}

impl CommandAction for LtoBackendAction {
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.spawn.get_arguments()
    }
}

impl ExecutableAction for LtoBackendAction {
    // Java: com/google/devtools/build/lib/rules/cpp/LtoBackendAction.java:179 LtoBackendAction.discoverInputs
    fn discover_inputs(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<NestedSet<Artifact>>, ActionExecutionException> {
        let imports_file_path = self.imports_input_path(action_execution_context);
        let lines = match super::cpp_compile_action::read_lines_as_latin1(&imports_file_path) {
            Ok(lines) => lines,
            Err(e) => {
                let message = format!(
                    "error reading imports file {}: {e}",
                    imports_file_path.get_path_string()
                );
                let code = create_failure_detail(
                    &message,
                    failure_details::lto_action::Code::ImportsReadIoException,
                );
                return Err(ActionExecutionException::with_cause(
                    message,
                    &e,
                    Some(self),
                    false,
                    Some(code),
                ));
            }
        };

        // Build set of files this LTO backend artifact will import from.
        // PORT: java-hash-order. Java's `HashSet<PathFragment>` only feeds a stable-order
        // NestedSetBuilder whose order Java never exposes for discovered inputs except through
        // `updateInputs` (a set); the first-seen order is kept here.
        let mut seen: HashSet<PathFragment> = HashSet::new();
        let mut import_set: Vec<PathFragment> = Vec::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let exec_path = PathFragment::create(&line);
            if exec_path.is_absolute() {
                let message = format!(
                    "Absolute paths not allowed in imports file {}: {}",
                    imports_file_path.get_path_string(),
                    exec_path.get_path_string()
                );
                let code = create_failure_detail(
                    &message,
                    failure_details::lto_action::Code::InvalidAbsolutePathInImports,
                );
                return Err(ActionExecutionException::new(
                    message,
                    Some(self),
                    false,
                    Some(code),
                ));
            }
            if seen.insert(exec_path.clone()) {
                import_set.push(exec_path);
            }
        }

        // Convert the import set of paths to the set of bitcode file artifacts.
        // Throws an error if there is any path in the importset that is not pat of any artifact
        let bitcode_input_set =
            self.compute_bitcode_inputs(&import_set, action_execution_context)?;
        let mut inputs = NestedSetBuilder::from_nested_set(&bitcode_input_set);
        inputs.add_transitive(&self.mandatory_inputs);
        self.update_inputs(inputs.build());
        Ok(Some(bitcode_input_set))
    }

    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        ExecutableAction::execute(&self.spawn, ctx)
    }
}

// PORT: skipped, a holder of static methods has no instance: LtoBackendArtifacts.<init>
/// `LtoBackendArtifacts.createLtoBackendActionForStarlark`.
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:177 LtoBackendArtifacts.createLtoBackendActionForStarlark
#[allow(clippy::too_many_arguments)]
pub fn create_lto_backend_action_for_starlark(
    owner: ActionOwner,
    configuration: Option<
        &dyn blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration,
    >,
    feature_configuration: Arc<FeatureConfiguration>,
    build_variables: CcToolchainVariablesRef,
    use_pic: bool,
    inputs: NestedSet<Artifact>,
    all_bitcode_files: Option<Arc<BitcodeFiles>>,
    imports: Option<Artifact>,
    outputs: Vec<Artifact>,
    env: Arc<ActionEnvironment>,
) -> LtoBackendAction {
    let command_lines =
        get_lto_backend_command_line(feature_configuration, build_variables, use_pic);
    LtoBackendAction::create(
        owner,
        configuration,
        inputs,
        all_bitcode_files,
        imports,
        outputs,
        command_lines,
        env,
    )
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:133 LtoBackendArtifacts.getLtoBackendCommandLine
fn get_lto_backend_command_line(
    feature_configuration: Arc<FeatureConfiguration>,
    build_variables: CcToolchainVariablesRef,
    use_pic: bool,
) -> CommandLines {
    let compiler = PathFragment::create(
        &feature_configuration.get_tool_path_for_action(CppActionNames::LTO_BACKEND),
    );
    let lto_command_line = LtoBackendCommandLine {
        feature_configuration,
        build_variables,
        use_pic,
    };
    let mut builder = CommandLines::builder();
    builder.add_single_path_fragment(compiler);
    builder.add_command_line(Arc::new(lto_command_line));
    builder.build()
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:42 LtoBackendArtifacts.addPathsToBuildVariablesBuilder
fn add_paths_to_build_variables_builder(
    build_variables_builder: &mut blaze_cc_features::cc_toolchain_variables::CcToolchainVariablesBuilder,
    index_path: Option<&str>,
    object_file_path: &str,
    dwo_file_path: Option<&str>,
    bitcode_file_path: &str,
) {
    use blaze_cc_features::cc_toolchain_variables::VariableObject;
    use blaze_cc_features::compile_build_variables::CompileBuildVariables;
    // Ideally, those strings would come directly from the execPath of the Artifacts of
    // the LtoBackendAction.Builder; however, in order to support tree artifacts, we need
    // the bitcodeFilePath to be different from the bitcodeTreeArtifact execPath.
    // The former is a file path and the latter is the directory path.
    // Therefore we accept strings as inputs rather than artifacts.
    match index_path {
        Some(index_path) => {
            build_variables_builder
                .add_variable("thinlto_index", VariableObject::string(index_path));
        }
        None => {
            // An empty input indicates not to perform cross-module optimization.
            build_variables_builder
                .add_variable("thinlto_index", VariableObject::string("/dev/null"));
        }
    }
    // The output from the LTO backend step is a native object file.
    build_variables_builder.add_variable(
        "thinlto_output_object_file",
        VariableObject::string(object_file_path),
    );
    // The input to the LTO backend step is the bitcode file.
    build_variables_builder.add_variable(
        "thinlto_input_bitcode_file",
        VariableObject::string(bitcode_file_path),
    );
    // Add the context sensitive instrument path to the backend.
    if let Some(dwo_file_path) = dwo_file_path {
        build_variables_builder.add_variable(
            CompileBuildVariables::PER_OBJECT_DEBUG_INFO_FILE.get_variable_name(),
            VariableObject::string(dwo_file_path),
        );
        build_variables_builder.add_variable(
            CompileBuildVariables::IS_USING_FISSION.get_variable_name(),
            VariableObject::string(""),
        );
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:73 LtoBackendArtifacts.getLtoBackendActionInputs
fn get_lto_backend_action_inputs(
    index: Option<&Artifact>,
    imports: Option<&Artifact>,
    bitcode_file: &Artifact,
    additional_inputs: &NestedSet<Artifact>,
) -> NestedSet<Artifact> {
    let mut inputs_builder = NestedSetBuilder::stable_order();
    inputs_builder.add_transitive(additional_inputs);
    inputs_builder.add(bitcode_file.clone());
    if let Some(imports) = imports {
        // Although the imports file is not used by the LTOBackendAction while the action is
        // executing, it is needed during the input discovery phase, and we must list it as an
        // input to the action in order for it to be preserved under
        // --discard_orphaned_artifacts.
        inputs_builder.add(imports.clone());
    }
    if let Some(index) = index {
        inputs_builder.add(index.clone());
    }
    inputs_builder.build()
}

// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:93 LtoBackendArtifacts.getLtoBackendActionOutputs
fn get_lto_backend_action_outputs(
    object_file: &Artifact,
    dwo_file: Option<&Artifact>,
) -> Vec<Artifact> {
    // ImmutableSet.Builder
    let mut builder = vec![object_file.clone()];
    // Add the context sensitive instrument path to the backend.
    if let Some(dwo_file) = dwo_file
        && !builder.contains(dwo_file)
    {
        builder.push(dwo_file.clone());
    }
    builder
}

/// `LtoBackendArtifacts.createLtoBackendActionForTemplate`.
// Java: com/google/devtools/build/lib/rules/cpp/LtoBackendArtifacts.java:191 LtoBackendArtifacts.createLtoBackendActionForTemplate
#[allow(clippy::too_many_arguments)]
pub fn create_lto_backend_action_for_template(
    action_owner: ActionOwner,
    additional_inputs: &NestedSet<Artifact>,
    env: Arc<ActionEnvironment>,
    build_variables: &CcToolchainVariablesRef,
    feature_configuration: Arc<FeatureConfiguration>,
    index: Option<&Artifact>,
    imports: Option<&Artifact>,
    bitcode_artifact: &Artifact,
    object_file: &Artifact,
    bitcode_files: Option<Arc<BitcodeFiles>>,
    dwo_file: Option<&Artifact>,
    use_pic: bool,
    bitcode_file_path: Option<&str>,
    is_dummy_action: bool,
) -> LtoBackendAction {
    check_state!(
        is_dummy_action
            || (index.is_none_or(|i| !i.is_tree_artifact())
                && imports.is_none_or(|i| !i.is_tree_artifact())
                && dwo_file.is_none_or(|d| !d.is_tree_artifact())
                && !object_file.is_tree_artifact()),
        "index, imports, object and dwo files cannot be TreeArtifacts. We need to know their exact \
         path not just directory path."
    );
    check_state!(
        is_dummy_action || (bitcode_artifact.is_tree_artifact() ^ bitcode_file_path.is_none()),
        "If bitcode file is a tree artifact, the bitcode file path must contain the path. If it is \
         not a tree artifact, then bitcode file path should be null to not override the path."
    );
    let mut build_variables_builder =
        blaze_cc_features::cc_toolchain_variables::CcToolchainVariables::builder_with_parent(Some(
            build_variables.clone(),
        ));
    let inputs = get_lto_backend_action_inputs(index, imports, bitcode_artifact, additional_inputs);
    let outputs = get_lto_backend_action_outputs(object_file, dwo_file);

    let index_path = index.map(|i| i.get_exec_path_string().to_owned());
    let dwo_file_path = dwo_file.map(|d| d.get_exec_path_string().to_owned());
    let bitcode_path = match bitcode_file_path {
        Some(p) => p.to_owned(),
        None => bitcode_artifact.get_exec_path_string().to_owned(),
    };
    add_paths_to_build_variables_builder(
        &mut build_variables_builder,
        index_path.as_deref(),
        object_file.get_exec_path_string(),
        dwo_file_path.as_deref(),
        &bitcode_path,
    );
    let build_variables_with_files = build_variables_builder.build();

    create_lto_backend_action_for_starlark(
        action_owner,
        /* configuration= */ None,
        feature_configuration,
        build_variables_with_files,
        use_pic,
        inputs,
        bitcode_files,
        imports.cloned(),
        outputs,
        env,
    )
}
