//! `CppCompileAction` and `CppCompileActionBuilder`: the action's inputs, outputs, environment,
//! execution info, command line, mnemonic and action key, and its execution half
//! (`discoverInputs`, `execute`, `.d` file parsing through `blaze_cc_features`' `DependencySet`
//! and `HeaderDiscovery`, include validation, `updateInputs`) as
//! `blaze_exec::actions::ExecutableAction`.
//!
//! PORT: include scanning (`--experimental_cc_include_scanning`), C++ module discovery after
//! include scanning and `/showIncludes` parsing (MSVC) are `unported!`.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::actions::command_lines::ParamFileActionInput;
use blaze_actions::actions::fingerprint_util::add_paths;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::path_mappers;
use blaze_analysis::platform::constraint_constants::get_os_from_constraints_or_host;
use blaze_analysis::platform::platform_info::PlatformInfo as AnalysisPlatformInfo;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragments::cpp_configuration::CppConfiguration;
use blaze_cc_features::actions as cc;
use blaze_cc_features::actions::NOOP as CC_NOOP;
use blaze_cc_features::cc_common::CoptsFilter;
use blaze_cc_features::cc_toolchain_features::FeatureConfiguration;
use blaze_cc_features::cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
use blaze_cc_features::compile_build_variables::CompileBuildVariables;
use blaze_cc_features::compile_command_line::CompileCommandLine;
use blaze_cc_features::cpp_action_names::CppActionNames;
use blaze_cc_features::cpp_rule_classes;
use blaze_cc_features::dependency_set::DependencySet;
use blaze_cc_features::header_discovery::{self, DiscoveringAction};
use blaze_cc_features::include_problems::IncludeProblems;
use blaze_cmdline::label_constants::{EXPERIMENTAL_EXTERNAL_PATH_PREFIX, EXTERNAL_PATH_PREFIX};
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::{ActionMetadataSnapshot, artifact_input};
use blaze_exec::actions::execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS;
use blaze_exec::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction,
    PathMapper as SpawnPathMapper, ResourceSet, ResourceSetOrBuilder, SimpleSpawn, SpawnResult,
};
use blaze_exec::analysis::actions::{get_input_path, to_action_input_set};
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_shell::ShellEscaper;
use blaze_util::{Fingerprint, check_argument, check_state};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::cc_artifact::{CcArtifact, CcPathMapper};
use super::cc_compilation_context::CcCompilationContext;
use super::cpp_file_types;
use super::cpp_include_extraction_context::{
    experimental_sibling_repository_layout, get_cpp_include_extraction_context,
};

/// `UUID.fromString("97493805-894f-493a-be66-9a698f45c31d")`: (least, most) significant bits.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:122 CppCompileAction.GUID
const GUID_LEAST: u64 = 0xbe66_9a69_8f45_c31d;
const GUID_MOST: u64 = 0x9749_3805_894f_493a;

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:124 CppCompileAction.CPP_COMPILE_MNEMONIC
pub const CPP_COMPILE_MNEMONIC: &str = "CppCompile";
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:125 CppCompileAction.OBJC_COMPILE_MNEMONIC
pub const OBJC_COMPILE_MNEMONIC: &str = "ObjcCompile";

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:130 ExecutionRequirements.REQUIRES_DARWIN
const REQUIRES_DARWIN: &str = "requires-darwin";

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:396 ExecutionRequirements.DIFFERENTIATE_WORKSPACE_CACHE
const DIFFERENTIATE_WORKSPACE_CACHE: &str = "internal-differentiate-workspace-cache";

fn cc_error(e: blaze_cc_features::actions::CommandLineExpansionException) -> MapFnError {
    MapFnError::CommandLineExpansion(CommandLineExpansionException::new(e.message))
}

/// The command line of `CompileCommandLine.getFilteredFeatureConfigurationCommandLine`: exactly
/// the same command line as the `CppCompileAction` (tool path and compiler options).
// Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:106 CompileCommandLine.getFilteredFeatureConfigurationCommandLine
struct FilteredFeatureConfigurationCommandLine {
    compile_command_line: CompileCommandLine,
    overwritten_variables: CcToolchainVariablesRef,
}

impl std::fmt::Debug for FilteredFeatureConfigurationCommandLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FilteredFeatureConfigurationCommandLine")
    }
}

impl blaze_actions::actions::command_line::CommandLine for FilteredFeatureConfigurationCommandLine {
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:110 CompileCommandLine.getFilteredFeatureConfigurationCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.compile_command_line
            .get_filtered_feature_configuration_command_line(Some(&*self.overwritten_variables))
            .map_err(cc_error)
    }
}

/// Action that represents some kind of C++ compilation step.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:120 CppCompileAction
pub struct CppCompileAction {
    base: AbstractAction,
    gcno_file: Option<Artifact>,
    source_file: Artifact,
    dotd_file: Option<Artifact>,
    configuration: Arc<BuildConfigurationValue>,
    mandatory_inputs: NestedSet<Artifact>,
    mandatory_spawn_inputs: NestedSet<Artifact>,
    allowed_derived_inputs: NestedSet<Artifact>,
    /// The set of input files that in addition to `CcCompilationContext#getDeclaredIncludeSrcs`
    /// need to be added to the set of input artifacts of the action if we don't use input
    /// discovery. They may be pruned after execution.
    additional_prunable_headers: NestedSet<Artifact>,
    grep_includes: Option<Artifact>,
    shareable: bool,
    should_scan_includes: bool,
    use_pic: bool,
    use_header_modules: bool,
    needs_include_validation: bool,
    cc_compilation_context: CcCompilationContext,
    builtin_include_files: Vec<Artifact>,
    // A list of files to include scan that are not source files, pcm files, or included via a
    // command-line "-include file.h". Actions that use non C++ files as source files--such as
    // Clif--may use this mechanism.
    additional_include_scanning_roots: Vec<Artifact>,
    pub compile_command_line: CompileCommandLine,
    /// The fingerprint of `compile_command_line`. This is computed lazily so that the command
    /// line is not unnecessarily flattened outside of action execution.
    command_line_key: OnceLock<Vec<u8>>,
    execution_info: IndexMap<String, String>,
    action_name: String,
    feature_configuration: Arc<FeatureConfiguration>,
    built_in_include_directories: Vec<PathFragment>,
    /// Set when the action prepares for execution. Used to preserve state between preparation
    /// and execution.
    additional_inputs: Mutex<Option<NestedSet<Artifact>>>,
    // PORT: `usedModules` is set only by the include-scanning path of `discoverInputs`, which is
    // `unported!`; without include scanning it is always null.
    used_cpp20_modules: Mutex<Option<Vec<Artifact>>>,
    inputs_discovered: AtomicBool,
    /// This field is set only for C++ module compiles (compiling .cppmap files into .pcm files).
    /// It stores the modules necessary for building this module as they will later also be
    /// required for building users of this module.
    discovered_modules: Mutex<Option<NestedSet<Artifact>>>,
    /// Used modules that are not transitively used through other topLevelModules.
    top_level_modules: Mutex<Option<NestedSet<Artifact>>>,
    param_file_action_input: Mutex<Option<ParamFileActionInput>>,
    param_file_path: Option<PathFragment>,
    module_files: Option<NestedSet<Artifact>>,
    modmap_input_file: Option<Artifact>,
    /// `configuration.getActionEnvironment()`.
    env: Arc<ActionEnvironment>,
    /// `PathMappers.getOutputPathsMode(configuration)`.
    output_paths_mode: OutputPathsMode,
    /// `getOwner().getExecutionPlatform()` as the analysis layer's `PlatformInfo`: the action
    /// layer's view of the owner's platform does not carry the constraints `getExecutionOs`
    /// reads. Set by `CppCompileActionBuilder`.
    execution_platform: Option<Arc<AnalysisPlatformInfo>>,
}

impl std::fmt::Debug for CppCompileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CppCompileAction")
            .field("source_file", &self.source_file)
            .field("outputs", &self.base.outputs())
            .finish()
    }
}

/// `configuration.getFragment(CppConfiguration.class)`.
fn cpp_configuration(configuration: &BuildConfigurationValue) -> &CppConfiguration {
    configuration
        .get_fragment::<CppConfiguration>()
        .expect("NullPointerException: configuration has no CppConfiguration")
}

/// `ActionEnvironment` of a configuration as the action layer's type.
fn action_environment(configuration: &BuildConfigurationValue) -> Arc<ActionEnvironment> {
    Arc::new(blaze_analysis::rule::action_environments::to_actions_env(
        configuration.get_action_environment(),
    ))
}

/// `PathMappers.getOutputPathsMode(configuration)`.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:145 PathMappers.getOutputPathsMode
fn get_output_paths_mode(configuration: &BuildConfigurationValue) -> OutputPathsMode {
    use blaze_optdefs::fragment::core_options::CoreOptionsOutputPathsMode;
    match blaze_analysis::rule::config_util::core_options(configuration).output_paths_mode {
        CoreOptionsOutputPathsMode::OFF => OutputPathsMode::OFF,
        CoreOptionsOutputPathsMode::STRIP => OutputPathsMode::STRIP,
    }
}

impl CppCompileAction {
    /// Creates a new action to compile C/C++ source files.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:243 CppCompileAction.<init>
    #[allow(clippy::too_many_arguments)]
    fn new(
        owner: ActionOwner,
        feature_configuration: Arc<FeatureConfiguration>,
        variables: CcToolchainVariablesRef,
        source_file: Artifact,
        configuration: Arc<BuildConfigurationValue>,
        shareable: bool,
        should_scan_includes: bool,
        use_pic: bool,
        use_header_modules: bool,
        mandatory_inputs: NestedSet<Artifact>,
        mandatory_spawn_inputs: NestedSet<Artifact>,
        builtin_include_files: Vec<Artifact>,
        additional_prunable_headers: NestedSet<Artifact>,
        output_file: Artifact,
        dotd_file: Option<Artifact>,
        diagnostics_file: Option<Artifact>,
        gcno_file: Option<Artifact>,
        dwo_file: Option<Artifact>,
        lto_indexing_file: Option<Artifact>,
        cc_compilation_context: CcCompilationContext,
        copts_filter: CoptsFilter,
        additional_include_scanning_roots: Vec<Artifact>,
        execution_info: IndexMap<String, String>,
        action_name: String,
        needs_include_validation: bool,
        built_in_include_directories: Vec<PathFragment>,
        grep_includes: Option<Artifact>,
        additional_outputs: Vec<Artifact>,
        module_files: Option<NestedSet<Artifact>>,
        modmap_input_file: Option<Artifact>,
    ) -> CppCompileAction {
        let base = AbstractAction::new(
            owner,
            mandatory_inputs.clone(),
            collect_outputs(
                output_file.clone(),
                dotd_file.as_ref(),
                diagnostics_file.as_ref(),
                gcno_file.as_ref(),
                dwo_file.as_ref(),
                lto_indexing_file.as_ref(),
                &additional_outputs,
            ),
        );
        let compile_command_line = build_command_line(
            copts_filter,
            &action_name,
            feature_configuration.clone(),
            variables,
        );
        let is_generate_dotd_file = cpp_file_types::header_discovery_required(&source_file)
            && !feature_configuration.is_enabled(cpp_rule_classes::PARSE_SHOWINCLUDES);
        let param_file_path =
            if feature_configuration.is_enabled(cpp_rule_classes::COMPILER_PARAM_FILE) {
                Some(
                    output_file
                        .get_exec_path()
                        .get_parent_directory()
                        .expect("output has a parent directory")
                        .get_child(&format!("{}.params", output_file.get_filename())),
                )
            } else {
                None
            };

        let mut allowed_derived_inputs_builder =
            NestedSetBuilder::from_nested_set(&mandatory_inputs);
        allowed_derived_inputs_builder
            .add_transitive(&additional_prunable_headers)
            .add_transitive(cc_compilation_context.get_declared_include_srcs())
            .add_transitive(cc_compilation_context.get_transitive_modules(use_pic))
            .add(source_file.clone());
        // The separate module is an allowed input to all compiles of this context except for its
        // own compile.
        if let Some(separate_module) = cc_compilation_context.get_separate_header_module(use_pic) {
            if *separate_module != base.outputs()[0] {
                allowed_derived_inputs_builder.add(separate_module.clone());
            }
        }
        if let Some(module_files) = &module_files {
            allowed_derived_inputs_builder.add_transitive(module_files);
        }
        let allowed_derived_inputs = allowed_derived_inputs_builder.build();

        let env = action_environment(&configuration);
        let output_paths_mode = get_output_paths_mode(&configuration);
        let mut action = CppCompileAction {
            base,
            gcno_file,
            source_file,
            dotd_file: if is_generate_dotd_file {
                dotd_file
            } else {
                None
            },
            configuration,
            mandatory_inputs,
            mandatory_spawn_inputs,
            allowed_derived_inputs,
            additional_prunable_headers,
            grep_includes,
            shareable,
            should_scan_includes,
            use_pic,
            use_header_modules,
            needs_include_validation,
            cc_compilation_context,
            builtin_include_files,
            additional_include_scanning_roots,
            compile_command_line,
            command_line_key: OnceLock::new(),
            execution_info,
            action_name,
            feature_configuration,
            built_in_include_directories,
            additional_inputs: Mutex::new(None),
            used_cpp20_modules: Mutex::new(None),
            inputs_discovered: AtomicBool::new(false),
            discovered_modules: Mutex::new(None),
            top_level_modules: Mutex::new(None),
            param_file_action_input: Mutex::new(None),
            param_file_path,
            module_files,
            modmap_input_file,
            env,
            output_paths_mode,
            execution_platform: None,
        };
        action.initialize_modules_upper_bounds_if_not_scanning_includes();
        action
    }

    /// Without include scanning, the exact set of modules used by this compilation is unknown,
    /// so use a suitable upper bound: any directly usable module may be used as a top-level
    /// module and any transitive module may be needed as an input.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:408 CppCompileAction.initializeModulesUpperBoundsIfNotScanningIncludes
    fn initialize_modules_upper_bounds_if_not_scanning_includes(&mut self) {
        if self.should_scan_includes || !self.use_header_modules {
            return;
        }
        let primary_output = self.get_primary_output();
        let separate = self
            .cc_compilation_context
            .get_separate_header_module(self.use_pic)
            .is_some_and(|m| *m == primary_output);
        let top_level_modules = self
            .cc_compilation_context
            .get_direct_modules(self.use_pic, separate);
        *self.top_level_modules.get_mut() = Some(top_level_modules.clone());
        if cpp_file_types::CPP_MODULE.matches_artifact(&primary_output)
            && !is_cpp20_module_compilation_action(&self.action_name)
        {
            let mut b = NestedSetBuilder::from_nested_set(
                self.cc_compilation_context
                    .get_transitive_modules(self.use_pic),
            );
            b.add_transitive(&top_level_modules);
            *self.discovered_modules.get_mut() = Some(b.build());
        }
    }

    fn cpp_configuration(&self) -> &CppConfiguration {
        cpp_configuration(&self.configuration)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:480 CppCompileAction.useInMemoryDotdFiles
    fn use_in_memory_dotd_files(&self) -> bool {
        self.cpp_configuration().get_inmemory_dotd_files()
    }

    /// Whether we should do "include scanning".
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:476 CppCompileAction.shouldScanIncludes
    pub fn should_scan_includes(&self) -> bool {
        self.should_scan_includes
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:494 CppCompileAction.getBuiltInIncludeDirectories
    pub fn get_built_in_include_directories(&self) -> &[PathFragment] {
        &self.built_in_include_directories
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:500 CppCompileAction.getBuiltInIncludeFiles
    pub fn get_built_in_include_files(&self) -> &[Artifact] {
        &self.builtin_include_files
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:792 CppCompileAction.getSourceFile
    pub fn get_source_file(&self) -> &Artifact {
        &self.source_file
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:798 CppCompileAction.getGrepIncludes
    pub fn get_grep_includes(&self) -> Option<&Artifact> {
        self.grep_includes.as_ref()
    }

    /// Set by `discoverInputs` or, when include scanning is disabled, in the constructor.
    /// Returns an empty `NestedSet`, if this is not a compile action producing a C++ module.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:807 CppCompileAction.getDiscoveredModules
    pub fn get_discovered_modules(&self) -> NestedSet<Artifact> {
        self.discovered_modules
            .lock()
            .clone()
            .unwrap_or_else(|| NestedSetBuilder::empty_set(Order::STABLE_ORDER))
    }

    /// Returns the path where the compiler should put the discovered dependency information.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:813 CppCompileAction.getDotdFile
    pub fn get_dotd_file(&self) -> Option<&Artifact> {
        self.dotd_file.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:817 CppCompileAction.getCcCompilationContext
    pub fn get_cc_compilation_context(&self) -> &CcCompilationContext {
        &self.cc_compilation_context
    }

    /// Returns explicitly listed header files.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1341 CppCompileAction.getDeclaredIncludeSrcs
    pub fn get_declared_include_srcs(&self) -> &NestedSet<Artifact> {
        self.cc_compilation_context.get_declared_include_srcs()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2062 CppCompileAction.getCompileCommandLine
    pub fn get_compile_command_line(&self) -> &CompileCommandLine {
        &self.compile_command_line
    }

    pub fn get_additional_include_scanning_roots(&self) -> &[Artifact] {
        &self.additional_include_scanning_roots
    }

    pub fn needs_include_validation(&self) -> bool {
        self.needs_include_validation
    }

    pub fn get_module_files(&self) -> Option<&NestedSet<Artifact>> {
        self.module_files.as_ref()
    }

    pub fn get_modmap_input_file(&self) -> Option<&Artifact> {
        self.modmap_input_file.as_ref()
    }

    pub fn get_gcno_file(&self) -> Option<&Artifact> {
        self.gcno_file.as_ref()
    }

    /// Returns the compiler options.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1108 CppCompileAction.getCompilerOptions
    pub fn get_compiler_options(&self) -> Result<Vec<String>, CommandLineError> {
        self.compile_command_line
            .get_compiler_options(None, &CC_NOOP)
            .map_err(cc_error)
    }

    /// Extracts all module (.pcm) files from potentialModules and returns a Variables object
    /// where their exec paths are added to the value "module_files".
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1263 CppCompileAction.calculateModuleVariable
    fn calculate_module_variable(
        potential_modules: &NestedSet<Artifact>,
    ) -> CcToolchainVariablesRef {
        let mut used_module_paths: Vec<String> = Vec::new();
        for input in potential_modules.to_list().iter() {
            if cpp_file_types::CPP_MODULE.matches_artifact(input) {
                used_module_paths.push(input.get_exec_path_string().to_owned());
            }
        }
        let mut variable_builder = CcToolchainVariables::builder();
        variable_builder.add_string_sequence_variable(
            CompileBuildVariables::MODULE_FILES.get_variable_name(),
            used_module_paths,
        );
        variable_builder.build()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1277 CppCompileAction.getOverwrittenVariables
    fn get_overwritten_variables(&self) -> CcToolchainVariablesRef {
        if self.use_header_modules {
            // There are three cases for when this method might be called:
            // 1. After input discovery, after which topLevelModules is set (in discoverInputs()).
            // 2. Without include scanning, in which case topLevelModules is set to an upper bound
            //    in the constructor.
            // 3. After the action is loaded from the local action cache, leaving topLevelModules
            //    null in the case of include scanning.
            match self.get_top_level_modules() {
                Some(modules) => Self::calculate_module_variable(&modules),
                None => Self::calculate_module_variable(&self.get_inputs()),
            }
        } else {
            CcToolchainVariables::builder().build()
        }
    }

    /// The `argv` without a parameter file (Java `getStarlarkArgv`).
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1037 CppCompileAction.getStarlarkArgv
    pub fn get_starlark_argv(&self) -> Result<Vec<String>, CommandLineError> {
        let overwritten = self.get_overwritten_variables();
        self.compile_command_line
            .get_arguments(None, Some(&*overwritten), &CC_NOOP)
            .map_err(cc_error)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1463 CppCompileAction.getCommandLineKey
    fn get_command_line_key(&self) -> Result<&[u8], CommandLineError> {
        if let Some(key) = self.command_line_key.get() {
            return Ok(key);
        }
        // For the argv part of the cache key, ignore all compiler flags that explicitly denote
        // module file (.pcm) inputs. Depending on input discovery, some of the unused ones are
        // removed from the command line. However, these actually don't have an influence on the
        // compile itself and so ignoring them for the cache key calculation does not affect
        // correctness. The compile itself is fully determined by the input source files and
        // module maps.
        let key = compute_command_line_key(&self.get_compiler_options()?);
        Ok(self.command_line_key.get_or_init(|| key))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1669 CppCompileAction.shouldParseShowIncludes
    fn should_parse_show_includes(&self) -> bool {
        self.feature_configuration
            .is_enabled(cpp_rule_classes::PARSE_SHOWINCLUDES)
    }
}

/// Collects the outputs: the object file, the coverage notes, additional outputs, the .d file,
/// the diagnostics file, the .dwo file and the LTO indexing file, in that order.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:425 CppCompileAction.collectOutputs
fn collect_outputs(
    output_file: Artifact,
    dotd_file: Option<&Artifact>,
    diagnostics_file: Option<&Artifact>,
    gcno_file: Option<&Artifact>,
    dwo_file: Option<&Artifact>,
    lto_indexing_file: Option<&Artifact>,
    additional_outputs: &[Artifact],
) -> Vec<Artifact> {
    let mut outputs = vec![output_file];
    if let Some(f) = gcno_file {
        outputs.push(f.clone());
    }
    outputs.extend(additional_outputs.iter().cloned());
    if let Some(f) = dotd_file {
        outputs.push(f.clone());
    }
    if let Some(f) = diagnostics_file {
        outputs.push(f.clone());
    }
    if let Some(f) = dwo_file {
        outputs.push(f.clone());
    }
    if let Some(f) = lto_indexing_file {
        outputs.push(f.clone());
    }
    outputs
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:454 CppCompileAction.buildCommandLine
pub(crate) fn build_command_line(
    copts_filter: CoptsFilter,
    action_name: &str,
    feature_configuration: Arc<FeatureConfiguration>,
    variables: CcToolchainVariablesRef,
) -> CompileCommandLine {
    let mut b = CompileCommandLine::builder(copts_filter, action_name);
    b.set_feature_configuration(feature_configuration)
        .set_variables(variables);
    b.build()
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1477 CppCompileAction.computeCommandLineKey
pub fn compute_command_line_key(compiler_options: &[String]) -> Vec<u8> {
    let mut fp = Fingerprint::new();
    fp.add_strings(compiler_options.iter());
    fp.digest_and_reset()
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1993 CppCompileAction.actionNameToMnemonic
pub fn action_name_to_mnemonic(
    action_name: &str,
    feature_configuration: &FeatureConfiguration,
    use_cpp_compile_header_mnemonic: bool,
) -> String {
    match action_name {
        CppActionNames::OBJC_COMPILE | CppActionNames::OBJCPP_COMPILE => {
            OBJC_COMPILE_MNEMONIC.to_owned()
        }
        // When compiling shared native deps, e.g. when two java_binary rules have the same set
        // of native dependencies, the CppCompileAction for link stamp data is shared also. This
        // means that out of two CppCompileAction instances, only one is actually executed, which
        // means that if extra actions are attached to both, one of the extra actions will find a
        // CppCompileAction for which discoverInputs() hasn't been called and thus trigger an
        // assertion. As a band-aid, change the mnemonic of said actions so that one can attach
        // extra actions to regular CppCompileActions without tickling this bug.
        CppActionNames::LINKSTAMP_COMPILE => "CppLinkstampCompile".to_owned(),
        CppActionNames::CPP_HEADER_PARSING => {
            let suffix = if use_cpp_compile_header_mnemonic {
                "Header"
            } else {
                ""
            };
            if feature_configuration.is_enabled(cpp_rule_classes::LANG_OBJC) {
                format!("{OBJC_COMPILE_MNEMONIC}{suffix}")
            } else {
                format!("{CPP_COMPILE_MNEMONIC}{suffix}")
            }
        }
        CppActionNames::CPP_HEADER_ANALYSIS => "CppHeaderAnalysis".to_owned(),
        CppActionNames::CPP_MODULE_DEPS_SCANNING => "CppDepsScanning".to_owned(),
        _ => CPP_COMPILE_MNEMONIC.to_owned(),
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2121 CppCompileAction.isCpp20ModuleCompilationAction
pub fn is_cpp20_module_compilation_action(action_name: &str) -> bool {
    action_name == CppActionNames::CPP20_MODULE_COMPILE
        || action_name == CppActionNames::CPP20_MODULE_CODEGEN
}

/// Separated into a helper method so that it can be called from CppCompileActionTemplate.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1413 CppCompileAction.computeKey
#[allow(clippy::too_many_arguments)]
pub fn compute_key_static(
    action_key_context: &ActionKeyContext,
    fp: &mut Fingerprint,
    env: &ActionEnvironment,
    environment_variables: &IndexMap<String, String>,
    execution_info: &IndexMap<String, String>,
    command_line_key: &[u8],
    declared_include_srcs: &NestedSet<Artifact>,
    mandatory_inputs: &NestedSet<Artifact>,
    mandatory_spawn_inputs: &NestedSet<Artifact>,
    prunable_headers: &NestedSet<Artifact>,
    built_in_include_directories: &[PathFragment],
    inputs_for_invalidation: &NestedSet<Artifact>,
    mnemonic: &str,
    output_paths_mode: OutputPathsMode,
) -> Result<(), ComputeKeyError> {
    fp.add_uuid(GUID_LEAST as i64, GUID_MOST as i64);
    env.add_to(fp);
    fp.add_string_map(
        environment_variables
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    );
    fp.add_string_map(execution_info.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    fp.add_bytes(command_line_key);

    action_key_context.add_nested_set_to_fingerprint(fp, declared_include_srcs)?;
    fp.add_int(0); // mark the boundary between input types
    action_key_context.add_nested_set_to_fingerprint(fp, mandatory_inputs)?;
    action_key_context.add_nested_set_to_fingerprint(fp, mandatory_spawn_inputs)?;
    fp.add_int(0);
    action_key_context.add_nested_set_to_fingerprint(fp, prunable_headers)?;

    // getArguments() above captures all changes which affect the compilation command and hence
    // the contents of the object file. But we need to also make sure that we re-execute the
    // action if any of the fields that affect whether validateInclusions will report an error or
    // warning have changed, otherwise we might miss some errors.
    add_paths(fp, built_in_include_directories);

    // This is needed for CppLinkstampCompile.
    fp.add_int(0);
    action_key_context.add_nested_set_to_fingerprint(fp, inputs_for_invalidation)?;

    path_mappers::add_to_fingerprint(
        mnemonic,
        execution_info,
        &NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        action_key_context,
        output_paths_mode,
        fp,
    )
}

impl Action for CppCompileAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2027 CppCompileAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        action_name_to_mnemonic(
            &self.action_name,
            &self.feature_configuration,
            self.cpp_configuration().use_cpp_compile_header_mnemonic(),
        )
    }

    /// For actions that discover inputs, the key must include input names.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1389 CppCompileAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_boolean(self.get_dotd_file().is_some() && self.use_in_memory_dotd_files());
        let environment_variables = self
            .compile_command_line
            .get_environment(&CC_NOOP)
            .map_err(|e| {
                ComputeKeyError::CommandLineExpansion(CommandLineExpansionException::new(e.message))
            })?;
        let command_line_key = self.get_command_line_key()?.to_vec();
        compute_key_static(
            action_key_context,
            fp,
            &self.env,
            &environment_variables,
            &self.execution_info,
            &command_line_key,
            self.cc_compilation_context.get_declared_include_srcs(),
            &self.mandatory_inputs,
            &self.mandatory_spawn_inputs,
            &self.additional_prunable_headers,
            &self.built_in_include_directories,
            self.cc_compilation_context.get_declared_include_srcs(),
            &self.get_mnemonic(),
            self.output_paths_mode,
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1068 CppCompileAction.getExtraActionInfo
    fn get_extra_action_info(
        &self,
        action_key_context: &ActionKeyContext,
    ) -> Result<blaze_protos::blaze::ExtraActionInfo, CommandLineError> {
        use blaze_protos::blaze::{CppCompileInfo, EnvironmentVariable};
        let mut info = CppCompileInfo {
            tool: Some(self.compile_command_line.get_tool_path()),
            ..Default::default()
        };

        let overwritten = self.get_overwritten_variables();
        let options = self
            .compile_command_line
            .get_compiler_options(Some(&*overwritten), &CC_NOOP)
            .map_err(cc_error)?;

        for option in options {
            info.compiler_option.push(option);
        }
        info.output_file = Some(self.get_primary_output().get_exec_path_string().to_owned());
        info.source_file = Some(self.get_source_file().get_exec_path_string().to_owned());
        if self.inputs_known() {
            for input in self.get_inputs().to_list().iter() {
                info.sources_and_headers
                    .push(input.get_exec_path_string().to_owned());
            }
        } else {
            info.sources_and_headers
                .push(self.get_source_file().get_exec_path_string().to_owned());
            for header in self
                .cc_compilation_context
                .get_declared_include_srcs()
                .to_list()
                .iter()
            {
                info.sources_and_headers
                    .push(header.get_exec_path_string().to_owned());
            }
        }
        // TODO(ulfjack): Extra actions currently ignore the client environment.
        for (name, value) in self
            .get_effective_environment(/* client_env= */ &IndexMap::new())
            .map_err(CommandLineError::CommandLineExpansion)?
        {
            info.variable.push(EnvironmentVariable { name, value });
        }

        let mut result = match blaze_actions::actions::action::abstract_get_extra_action_info(
            self,
            action_key_context,
        ) {
            Ok(result) => result,
            Err(CommandLineError::CommandLineExpansion(e)) => panic!(
                "AssertionError: CppCompileAction command line expansion cannot fail: {}",
                e.message
            ),
            Err(e) => return Err(e),
        };
        result.cpp_compile_info = Some(info);
        Ok(result)
    }

    /// Without include scanning: the declared include sources and the additional prunable
    /// headers.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1927 CppCompileAction.getInputFilesForExtraAction
    fn get_input_files_for_extra_action(&self) -> Option<NestedSet<Artifact>> {
        if !self.should_scan_includes {
            let mut builder = NestedSetBuilder::from_nested_set(
                self.cc_compilation_context.get_declared_include_srcs(),
            );
            builder.add_transitive(&self.additional_prunable_headers);
            return Some(builder.build());
        }
        blaze_util::unported!(
            "CppCompileAction.getInputFilesForExtraAction",
            "include scanning (findUsedHeaders)"
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:539 CppCompileAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        is_cpp20_module_compilation_action(&self.action_name)
            || self.should_scan_includes
            || self.get_dotd_file().is_some()
            || self.should_parse_show_includes()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:547 CppCompileAction.inputsDiscovered
    fn inputs_discovered(&self) -> bool {
        self.inputs_discovered.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:552 CppCompileAction.setInputsDiscovered
    fn set_inputs_discovered(&self, inputs_discovered: bool) {
        self.inputs_discovered
            .store(inputs_discovered, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1305 CppCompileAction.getAllowedDerivedInputs
    fn get_allowed_derived_inputs(&self) -> NestedSet<Artifact> {
        self.allowed_derived_inputs.clone()
    }

    /// If this is compiling a module, restores the value of `discoveredModules`, which is used
    /// to create the `ActionExecutionValue` after an action cache hit.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1317 CppCompileAction.updateInputs
    fn update_inputs(&self, inputs: NestedSet<Artifact>) {
        // super.updateInputs(inputs)
        self.abstract_update_inputs(inputs.clone());
        if cpp_file_types::CPP_MODULE.matches_artifact(&self.get_primary_output())
            && !is_cpp20_module_compilation_action(&self.action_name)
        {
            *self.discovered_modules.lock() = Some(NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                inputs
                    .to_list()
                    .iter()
                    .filter(|input| cpp_file_types::CPP_MODULE.matches_artifact(input))
                    .cloned(),
            ));
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:764 CppCompileAction.getOriginalInputs
    fn get_original_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:505 CppCompileAction.getMandatoryInputs
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        self.mandatory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:510 CppCompileAction.getMandatoryOutputs
    fn get_mandatory_outputs(&self) -> Vec<Artifact> {
        // Never prune orphaned modules files. To cut down critical paths, CppCompileActions do
        // not add modules files as inputs. Instead they rely on input discovery to recognize the
        // needed ones. However, orphan detection runs before input discovery and thus module
        // files would be discarded as orphans.
        let output_file = self.get_primary_output();
        if cpp_file_types::CPP_MODULE.matches_artifact(&output_file) {
            return vec![output_file];
        }
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1300 CppCompileAction.getSchedulingDependencies
    fn get_scheduling_dependencies(&self) -> NestedSet<Artifact> {
        self.cc_compilation_context
            .get_declared_include_srcs()
            .clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:489 CppCompileAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.env.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1011 CppCompileAction.getEffectiveEnvironment
    // PORT: the trait method has no PathMapper argument; PathMapper.NOOP is used, as in every
    // analysis-time caller.
    fn get_effective_environment(
        &self,
        client_env: &IndexMap<String, String>,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        let env = self.get_environment();
        let mut environment = IndexMap::with_capacity(env.estimated_size());
        env.resolve(&mut environment, client_env);

        if !self.get_execution_info().contains_key(REQUIRES_DARWIN) {
            // Linux: this prevents gcc/clang from writing the unpredictable (and often
            // irrelevant) value of getcwd() into the debug info. Not applicable to Darwin or
            // Windows, which have no /proc.
            environment.insert("PWD".to_owned(), "/proc/self/cwd".to_owned());
        }

        let compile_env = self
            .compile_command_line
            .get_environment(&CC_NOOP)
            .map_err(|e| CommandLineExpansionException::new(e.message))?;
        for (k, v) in compile_env {
            environment.insert(k, v);
        }
        Ok(environment)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:787 CppCompileAction.getPrimaryInput
    fn get_primary_input(&self) -> Option<Artifact> {
        Some(self.source_file.clone())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1383 CppCompileAction.isShareable
    fn is_shareable(&self) -> bool {
        self.shareable
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1330 CppCompileAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        let prefix = match self.action_name.as_str() {
            CppActionNames::CPP_HEADER_ANALYSIS => "Header analysis for ",
            CppActionNames::CPP_MODULE_DEPS_SCANNING => "Deps scanning for ",
            _ => "Compiling ",
        };
        Some(format!("{prefix}{}", self.source_file.pretty_print()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1113 CppCompileAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        blaze_actions::actions::action::merge_maps(
            &self.get_exec_properties(),
            &self.execution_info,
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2033 CppCompileAction.describeKey
    fn describe_key(&self) -> Option<String> {
        let mut message = String::new();
        message.push_str(
            &self
                .get_progress_message(None)
                .unwrap_or_else(|| "null".to_owned()),
        );
        message.push('\n');
        // Outputting one argument per line makes it easier to diff the results.
        // The first element in getArguments() is actually the command to execute.
        let mut legend = "  Command: ";
        match self.get_arguments() {
            Ok(arguments) => {
                for argument in ShellEscaper::escape_all(&arguments) {
                    message.push_str(legend);
                    message.push_str(&argument);
                    message.push('\n');
                    legend = "  Argument: ";
                }
            }
            Err(e) => {
                message.push_str("  Could not expand command line: ");
                // PORT: Java appends the exception's toString().
                match e {
                    MapFnError::CommandLineExpansion(e) => message.push_str(&format!(
                        "com.google.devtools.build.lib.actions.CommandLineExpansionException: {}",
                        e.message
                    )),
                    MapFnError::Interrupted(_) => {
                        message.push_str("java.lang.InterruptedException")
                    }
                }
                message.push('\n');
            }
        }

        for src in self.get_declared_include_srcs().to_list().iter() {
            message.push_str("  Declared include source: ");
            message.push_str(&ShellEscaper::escape_string(src.get_exec_path_string()));
            message.push('\n');
        }

        Some(message)
    }

    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1037 CppCompileAction.getStarlarkArgv
    fn get_starlark_argv(
        &self,
    ) -> Result<Option<Vec<String>>, blaze_actions::actions::action::ActionApiError> {
        match CppCompileAction::get_starlark_argv(self) {
            Ok(argv) => Ok(Some(argv)),
            // Java: catch (CommandLineExpansionException ex) { throw new EvalException(ex); }
            Err(CommandLineError::CommandLineExpansion(e)) => Err(
                blaze_actions::actions::action::ActionApiError::Eval(e.message),
            ),
            Err(CommandLineError::Interrupted(e)) => Err(
                blaze_actions::actions::action::ActionApiError::Interrupted(e),
            ),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1049 CppCompileAction.getStarlarkArgs
    fn get_starlark_args(
        &self,
    ) -> Option<Vec<blaze_actions::analysis::starlark::args::RegisteredArgs>> {
        // ImmutableSet of the directory inputs, in getInputs().toList() order.
        let mut directory_inputs: indexmap::IndexSet<Artifact> = indexmap::IndexSet::new();
        for input in self.get_inputs().to_list().iter() {
            if input.is_directory() {
                directory_inputs.insert(input.clone());
            }
        }

        let command_line: blaze_actions::actions::command_line::CommandLineRef =
            Arc::new(FilteredFeatureConfigurationCommandLine {
                compile_command_line: self.compile_command_line.clone(),
                // PORT: Java's anonymous CommandLine asks the action for getOverwrittenVariables()
                // each time it is expanded; the action is not reachable from an owned command
                // line here, so the variables are taken when the Args is made.
                overwritten_variables: self.get_overwritten_variables(),
            });
        let mut param_file_info = None;
        if self.cpp_configuration().use_args_params_file() {
            param_file_info = Some(
                blaze_actions::actions::param_file_info::ParamFileInfo::builder(
                    ParameterFileType::GCC_QUOTED,
                )
                .set_use_always(true)
                .build(),
            );
        }

        let args = blaze_actions::analysis::starlark::args::RegisteredArgs::for_registered_action(
            command_line,
            param_file_info,
            directory_inputs.into_iter().collect(),
        );

        Some(vec![args])
    }
}

impl CommandAction for CppCompileAction {
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1028 CppCompileAction.getArguments
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        let overwritten = self.get_overwritten_variables();
        self.compile_command_line
            .get_arguments(self.param_file_path.as_ref(), Some(&*overwritten), &CC_NOOP)
            .map_err(cc_error)
    }
}

// -------------------------------------------------------------------------------------------
// Execution

/// `createFailureDetail(message, detailedCode)` (`createDetailedExitCode` wraps it in
/// `DetailedExitCode.of`, which the execution layer derives from the detail).
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2104 CppCompileAction.createDetailedExitCode
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2108 CppCompileAction.createFailureDetail
fn create_failure_detail(
    message: &str,
    detailed_code: failure_details::cpp_compile::Code,
) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::CppCompile(failure_details::CppCompile {
            code: detailed_code as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:919 FileSystemUtils.startsWithAny
fn starts_with_any(path: &PathFragment, prefixes: &[PathFragment]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}

/// `System.err.println(s)`: the server's default-charset (ISO-8859-1) `PrintStream`, so an
/// internal string leaves as its bytes.
// TODO(byte-strings lead): move to blaze_util
fn system_err_println(s: &str) {
    let mut bytes = blaze_util::string_encoding::latin1_encode_lossy(s).into_owned();
    bytes.push(b'\n');
    let _ = blaze_events::io::system_err().write(&bytes);
}

/// Java `String.trim()`: removes leading and trailing characters `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `MSVC_CHARS`: `[mM] [sS] [vV] [cC]`.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:910 CppCompileAction.MSVC_CHARS
const MSVC_CHARS: [&[u8]; 4] = [b"mM", b"sS", b"vV", b"cC"];
/// `INCLUDE_PREFIX_CHARS`: `[-/] [iI]`.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:916 CppCompileAction.INCLUDE_PREFIX_CHARS
const INCLUDE_PREFIX_CHARS: [&[u8]; 2] = [b"-/", b"iI"];

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:919 CppCompileAction.substrMatchesChars
fn substr_matches_chars(s: &[char], start_pos: usize, substr: &[&[u8]]) -> bool {
    for (i, chars) in substr.iter().enumerate() {
        let c = s[start_pos + i];
        if !chars.iter().any(|&b| char::from(b) == c) {
            return false;
        }
    }
    true
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:929 CppCompileAction.matchesCaseInsensitiveMsvc
fn matches_case_insensitive_msvc(s: &str) -> bool {
    // Java `length()` and `charAt` count the chars of the internal string.
    let chars: Vec<char> = s.chars().collect();
    chars.len() >= 4 && substr_matches_chars(&chars, 0, &MSVC_CHARS)
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:933 CppCompileAction.matchesIncludeCaseInsensitiveMsvc
fn matches_include_case_insensitive_msvc(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    chars.len() >= 6
        && substr_matches_chars(&chars, 0, &INCLUDE_PREFIX_CHARS)
        && substr_matches_chars(&chars, 2, &MSVC_CHARS)
}

/// The `-I` part of `getIncludeDirs()`: the include directories given in `copts`.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:843 CppCompileAction.getIncludeDirs
fn include_dirs_from_copts(copts: &[String], result: &mut Vec<PathFragment>) {
    for opt in copts {
        if opt.starts_with("-I") || opt.starts_with("/I") {
            // We insist on the combined form "-Idir".
            let include_dir = &opt[2..];
            if include_dir.is_empty() {
                continue;
            }
            if matches_case_insensitive_msvc(include_dir) {
                // This is actually a "-imsvc", a system include dir.
                continue;
            }
            result.push(PathFragment::create(&opt[2..]));
        }
    }
}

/// `getSystemIncludeDirs(compilerOptions)`; `pretty_print` is the action's `prettyPrint()` for
/// the dangling-flag warning.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:873 CppCompileAction.getSystemIncludeDirs
fn system_include_dirs_from_options(
    compiler_options: &[String],
    pretty_print: &dyn Fn() -> String,
) -> Vec<PathFragment> {
    // TODO(bazel-team): parsing the command line flags here couples us to gcc- and
    // clang-cl-style compiler command lines; use a different way to specify system includes (for
    // example through a system_includes attribute in cc_toolchain); note that that would disallow
    // users from specifying system include paths via the copts attribute.
    // Currently, this works together with the include_paths features because getCommandLine()
    // will get the system include paths from the {@code CcCompilationContext} instead.
    let mut result = Vec::new();
    let mut i = 0;
    while i < compiler_options.len() {
        let opt = &compiler_options[i];
        let system_include_flag: Option<&str> = if opt.starts_with("-isystem") {
            Some("-isystem")
        } else if matches_include_case_insensitive_msvc(opt) {
            Some(&opt[0..6])
        } else {
            None
        };
        let Some(system_include_flag) = system_include_flag else {
            i += 1;
            continue;
        };

        if opt.len() > system_include_flag.len() {
            result.push(PathFragment::create(java_trim(
                &opt[system_include_flag.len()..],
            )));
        } else if i + 1 < compiler_options.len() {
            i += 1;
            result.push(PathFragment::create(&compiler_options[i]));
        } else {
            system_err_println(&format!(
                "WARNING: dangling {} flag in options for {}",
                system_include_flag,
                pretty_print()
            ));
        }
        i += 1;
    }
    result
}

/// `(os, inputsSize) -> estimateResourceConsumptionLocal(enabled, mnemonic, os, inputsSize)`, the
/// spawn's local resources.
#[derive(Debug)]
struct CppCompileLocalResources {
    enabled: bool,
    mnemonic: String,
}

impl ResourceSetOrBuilder for CppCompileLocalResources {
    fn build_resource_set(&self, inputs_size: i32) -> ResourceSet {
        // PORT: the OS argument is the host OS; Blaze runs on Linux only.
        estimate_resource_consumption_local(
            self.enabled,
            Some(self.mnemonic.as_str()),
            cc::OS::LINUX,
            inputs_size,
        )
    }
}

/// `AbstractAction.DEFAULT_RESOURCE_SET`.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:78 AbstractAction.DEFAULT_RESOURCE_SET
fn default_resource_set() -> ResourceSet {
    ResourceSet::create_with_ram_cpu(
        blaze_actions::actions::action::DEFAULT_RESOURCE_SET_MEMORY_MB,
        blaze_actions::actions::action::DEFAULT_RESOURCE_SET_CPU,
    )
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1351 CppCompileAction.estimateResourceConsumptionLocal
fn estimate_resource_consumption_local(
    enabled: bool,
    mnemonic: Option<&str>,
    os: cc::OS,
    inputs: i32,
) -> ResourceSet {
    if !enabled {
        return default_resource_set();
    }

    let Some(mnemonic) = mnemonic else {
        return default_resource_set();
    };

    match mnemonic {
        CPP_COMPILE_MNEMONIC => match os {
            cc::OS::DARWIN | cc::OS::LINUX => ResourceSet::create_with_ram_cpu(
                /* memoryMb= */ 80.0 + 0.7 * f64::from(inputs),
                /* cpu= */ 1.0,
            ),
            _ => default_resource_set(),
        },
        OBJC_COMPILE_MNEMONIC => match os {
            cc::OS::DARWIN => ResourceSet::create_with_ram_cpu(
                /* memoryMb= */ 80.0 + 0.2 * f64::from(inputs),
                /* cpu= */ 1.0,
            ),
            _ => default_resource_set(),
        },
        _ => default_resource_set(),
    }
}

/// `ImmutableMap.Builder.put` followed by `buildOrThrow()`: a duplicate key throws.
fn put_or_throw(map: &mut IndexMap<String, String>, key: &str, value: String) {
    if let Some(existing) = map.get(key) {
        panic!(
            "IllegalArgumentException: Multiple entries with same key: {key}={value} and \
             {key}={existing}"
        );
    }
    map.insert(key.to_owned(), value);
}

/// `FileSystemUtils.readLinesAsLatin1(path)`: the lines of the file, split at `\n`, `\r` and
/// `\r\n` (Guava `CharSource.readLines`).
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:794 FileSystemUtils.readLinesAsLatin1
fn read_lines_as_latin1(path: &Path) -> std::io::Result<Vec<String>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    path.get_input_stream()?.read_to_end(&mut bytes)?;
    let text = blaze_util::string_encoding::latin1_decode(&bytes);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            c => {
                current.push(c);
                pending = true;
            }
        }
    }
    if pending {
        lines.push(current);
    }
    Ok(lines)
}

/// Maps the `blaze_cc_features` artifact handles that `HeaderDiscovery` works on back to the
/// real artifacts.
///
/// PORT: Java has one `Artifact` class; `blaze_cc_features::actions::Artifact` offers no
/// downcast, so each wrapped artifact is remembered here.
#[derive(Default)]
struct CcArtifactTable(Mutex<HashMap<cc::ArtifactRef, Artifact>>);

impl CcArtifactTable {
    fn wrap(&self, artifact: &Artifact) -> cc::ArtifactRef {
        let r = cc::ArtifactRef(Arc::new(CcArtifact::new(artifact.clone())));
        self.0
            .lock()
            .entry(r.clone())
            .or_insert_with(|| artifact.clone());
        r
    }

    fn wrap_set(&self, set: &NestedSet<Artifact>) -> NestedSet<cc::ArtifactRef> {
        NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            set.to_list()
                .iter()
                .map(|a| self.wrap(a))
                .collect::<Vec<_>>(),
        )
    }

    fn unwrap(&self, r: &cc::ArtifactRef) -> Artifact {
        match self.0.lock().get(r) {
            Some(a) => a.clone(),
            None => panic!(
                "IllegalStateException: header discovery returned an unknown artifact {}",
                r.get_exec_path_string()
            ),
        }
    }
}

/// The compile action as `HeaderDiscovery` reads it.
struct CppDiscoveringAction<'a> {
    action: &'a CppCompileAction,
    table: &'a CcArtifactTable,
}

impl DiscoveringAction for CppDiscoveringAction<'_> {
    fn get_owner_label(&self) -> String {
        self.action.owner_label_string()
    }

    fn get_owner_repository_name(&self) -> String {
        match self.action.get_owner().get_label() {
            Some(label) => label.get_repository().get_name().to_owned(),
            None => panic!("NullPointerException: action.getOwner().getLabel()"),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/HeaderDiscovery.java:161 HeaderDiscovery.discoverInputsFromDependencies
    fn get_execution_os(&self) -> cc::OS {
        // Java: ConstraintConstants.getOsFromConstraintsOrHost(action.getExecutionPlatform()).
        match get_os_from_constraints_or_host(self.action.execution_platform.as_deref()) {
            blaze_shell::util::os::OS::Darwin => cc::OS::DARWIN,
            blaze_shell::util::os::OS::Freebsd => cc::OS::FREEBSD,
            blaze_shell::util::os::OS::Openbsd => cc::OS::OPENBSD,
            blaze_shell::util::os::OS::Linux => cc::OS::LINUX,
            blaze_shell::util::os::OS::Windows => cc::OS::WINDOWS,
            blaze_shell::util::os::OS::Unknown => cc::OS::UNKNOWN,
        }
    }

    fn get_inputs(&self) -> NestedSet<cc::ArtifactRef> {
        self.table.wrap_set(&self.action.get_inputs())
    }
}

/// The `ArtifactFactory` as the `blaze_cc_features` `ArtifactResolver`.
struct CcArtifactResolver<'a> {
    factory: &'a ArtifactFactory,
    table: &'a CcArtifactTable,
}

impl cc::ArtifactResolver for CcArtifactResolver<'_> {
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:570 ArtifactFactory.resolveSourceArtifact
    fn resolve_source_artifact(
        &self,
        exec_path: &PathFragment,
        repository_name: &blaze_cmdline::RepositoryName,
    ) -> Option<cc::ArtifactRef> {
        self.factory
            .resolve_source_artifact(exec_path, repository_name)
            .map(|a| self.table.wrap(&a))
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:576 ArtifactFactory.resolveSourceArtifactsAsciiCaseInsensitively
    fn resolve_source_artifacts_ascii_case_insensitively(
        &self,
        exec_path: &PathFragment,
        repository_name: &blaze_cmdline::RepositoryName,
    ) -> Vec<cc::ArtifactRef> {
        self.factory
            .resolve_source_artifacts_ascii_case_insensitively(exec_path, repository_name)
            .iter()
            .map(|a| self.table.wrap(a))
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:722 ArtifactFactory.isDerivedArtifact
    fn is_derived_artifact(&self, exec_path: &PathFragment) -> bool {
        self.factory.is_derived_artifact(exec_path)
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1117 CppCompileAction.validateInclude
#[allow(clippy::mutable_key_type)]
fn validate_include(
    allowed_includes: &HashSet<Artifact>,
    ignore_dirs: &[PathFragment],
    include: &Artifact,
) -> bool {
    // Only declared modules are added to an action and so they are always valid.
    cpp_file_types::CPP_MODULE.matches_artifact(include)
        ||
        // TODO(b/145253507): Exclude objc module maps from check, due to bad interaction with
        // local_objc_modules feature.
        cpp_file_types::OBJC_MODULE_MAP.matches_artifact(include)
        ||
        // It's a declared include/
        allowed_includes.contains(include)
        ||
        // Ignore headers from built-in include directories.
        starts_with_any(include.get_exec_path(), ignore_dirs)
}

// The execution layer's `ActionExecutionException` carries the Java exception's fields (large).
#[allow(clippy::result_large_err)]
impl CppCompileAction {
    /// `getOwner().getLabel()` as Java's `String.valueOf`.
    fn owner_label_string(&self) -> String {
        self.get_owner()
            .get_label()
            .map_or_else(|| "null".to_owned(), |l| l.to_string())
    }

    /// The `ActionExecutionException` thrown for a `CommandLineExpansionException` while building
    /// the compile command line.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:676 CppCompileAction.discoverInputs
    fn command_generation_failure(&self, cause_message: &str) -> ActionExecutionException {
        let message = format!(
            "failed to generate compile command for rule '{}: {}",
            self.owner_label_string(),
            cause_message
        );
        let code = create_failure_detail(
            &message,
            failure_details::cpp_compile::Code::CommandGenerationFailure,
        );
        ActionExecutionException::new(
            message,
            Some(self),
            /* catastrophe= */ false,
            Some(code),
        )
    }

    /// A `blaze_cc_features` include-check exception as the execution layer's
    /// `ActionExecutionException` (`new ActionExecutionException(message, action, catastrophe,
    /// createDetailedExitCode(message, code))`).
    // Java: com/google/devtools/build/lib/rules/cpp/IncludeProblems.java:43 IncludeProblems.assertProblemFree
    fn from_cc_exception(&self, e: cc::ActionExecutionException) -> ActionExecutionException {
        let code = match e.code {
            cc::CppCompileCode::UNDECLARED_INCLUSIONS => {
                failure_details::cpp_compile::Code::UndeclaredInclusions
            }
            cc::CppCompileCode::COMMAND_GENERATION_FAILURE => {
                failure_details::cpp_compile::Code::CommandGenerationFailure
            }
        };
        let detail = create_failure_detail(&e.message, code);
        ActionExecutionException::new(e.message, Some(self), e.catastrophe, Some(detail))
    }

    /// Returns the list of additional inputs found by dependency discovery, during action
    /// preparation. `discover_inputs` must be called before this method is called on each action
    /// execution.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:529 CppCompileAction.getAdditionalInputs
    pub fn get_additional_inputs(&self) -> NestedSet<Artifact> {
        self.additional_inputs
            .lock()
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: additionalInputs"))
    }

    /// Clears the discovered `additionalInputs`.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:534 CppCompileAction.clearAdditionalInputs
    fn clear_additional_inputs(&self) {
        *self.additional_inputs.lock() = None;
    }

    /// Make sure to clear the additional inputs potentially left over from an old build (in case
    /// we ran discoverInputs, but not beginExecution).
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:651 CppCompileAction.prepareInputDiscovery
    pub fn prepare_input_discovery(&self) {
        self.clear_additional_inputs();
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:569 CppCompileAction.getTopLevelModules
    fn get_top_level_modules(&self) -> Option<NestedSet<Artifact>> {
        self.top_level_modules.lock().clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:484 CppCompileAction.enabledCppCompileResourcesEstimation
    fn enabled_cpp_compile_resources_estimation(&self) -> bool {
        self.cpp_configuration()
            .get_experimental_cpp_compile_resources_estimation()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:822 CppCompileAction.getQuoteIncludeDirs
    pub fn get_quote_include_dirs(&self) -> Vec<PathFragment> {
        let mut result = self.cc_compilation_context.get_quote_include_dirs();
        let copts = self.compile_command_line.get_copts(&CC_NOOP);
        let mut i = 0;
        while i < copts.len() {
            let opt = &copts[i];
            if let Some(rest) = opt.strip_prefix("-iquote") {
                if opt.len() > 7 {
                    result.push(PathFragment::create(java_trim(rest)));
                } else if i + 1 < copts.len() {
                    i += 1;
                    result.push(PathFragment::create(&copts[i]));
                } else {
                    system_err_println(&format!(
                        "WARNING: dangling -iquote flag in options for {}",
                        self.pretty_print()
                    ));
                }
            }
            i += 1;
        }
        result
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:843 CppCompileAction.getIncludeDirs
    pub fn get_include_dirs(&self) -> Vec<PathFragment> {
        let mut result = self.cc_compilation_context.get_include_dirs();
        include_dirs_from_copts(&self.compile_command_line.get_copts(&CC_NOOP), &mut result);
        result
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:864 CppCompileAction.getFrameworkIncludeDirs
    pub fn get_framework_include_dirs(&self) -> Vec<PathFragment> {
        self.cc_compilation_context.get_framework_include_dirs()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:869 CppCompileAction.getSystemIncludeDirs
    pub fn get_system_include_dirs(&self) -> Result<Vec<PathFragment>, CommandLineError> {
        Ok(self.get_system_include_dirs_from(&self.get_compiler_options()?))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:873 CppCompileAction.getSystemIncludeDirs
    fn get_system_include_dirs_from(&self, compiler_options: &[String]) -> Vec<PathFragment> {
        system_include_dirs_from_options(compiler_options, &|| self.pretty_print())
    }

    /// `getArguments(pathMapper)`.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1032 CppCompileAction.getArguments
    fn get_arguments_with(
        &self,
        path_mapper: &dyn cc::PathMapper,
    ) -> Result<Vec<String>, cc::CommandLineExpansionException> {
        let overwritten = self.get_overwritten_variables();
        self.compile_command_line.get_arguments(
            self.param_file_path.as_ref(),
            Some(&*overwritten),
            path_mapper,
        )
    }

    /// Enforce that the includes actually visited during the compile were properly declared in
    /// the rules.
    ///
    /// The technique is to walk through all of the reported includes that gcc emits into the .d
    /// file, and verify that they came from acceptable relative include directories.
    ///
    /// Fails iff there was an undeclared dependency.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1152 CppCompileAction.validateInclusions
    // PORT: Java's unused `actionExecutionContext` parameter is dropped.
    #[allow(clippy::mutable_key_type)]
    pub fn validate_inclusions(
        &self,
        inputs_for_validation: &NestedSet<Artifact>,
    ) -> Result<(), ActionExecutionException> {
        if !self.needs_include_validation {
            return Ok(());
        }
        let mut errors = IncludeProblems::new();
        let mut allowed_includes: HashSet<Artifact> = HashSet::new();
        allowed_includes.extend(self.mandatory_inputs.to_list().iter().cloned());
        allowed_includes.extend(
            self.cc_compilation_context
                .get_declared_include_srcs()
                .to_list()
                .iter()
                .cloned(),
        );
        allowed_includes.extend(self.additional_prunable_headers.to_list().iter().cloned());

        let ignore_dirs = if self.cpp_configuration().is_strict_system_includes() {
            self.get_built_in_include_directories().to_vec()
        } else {
            self.get_validation_ignored_dirs()
        };

        // Copy the nested sets to hash sets for fast contains checking, but do so lazily.
        // Avoid immutable sets here to limit memory churn.
        for input in inputs_for_validation.to_list().iter() {
            if !validate_include(&allowed_includes, &ignore_dirs, input) {
                errors.add(input.get_exec_path().get_path_string());
            }
        }
        errors
            .assert_problem_free(&format!(
                "undeclared inclusion(s) in rule '{}':\nthis rule is missing dependency \
                 declarations for the following files included by '{}':",
                self.owner_label_string(),
                self.get_source_file().pretty_print()
            ))
            .map_err(|e| self.from_cc_exception(e))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1187 CppCompileAction.getValidationIgnoredDirs
    fn get_validation_ignored_dirs(&self) -> Vec<PathFragment> {
        let mut dirs = self.get_built_in_include_directories().to_vec();
        dirs.extend(self.cc_compilation_context.get_system_include_dirs());
        dirs.extend(self.cc_compilation_context.get_external_include_dirs());
        dirs
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1196 CppCompileAction.verifyActionIncludePaths
    pub fn verify_action_include_paths(
        &self,
        system_include_dirs: &[PathFragment],
        sibling_repository_layout: bool,
    ) -> Result<(), ActionExecutionException> {
        let ignored_dirs_list = self.get_validation_ignored_dirs();
        let ignored_dirs: HashSet<&PathFragment> = ignored_dirs_list.iter().collect();
        // We currently do not check the output of:
        // - getBuiltinIncludeDirs(): while in practice this doesn't happen, bazel can be
        //   configured to use an absolute system root, in which case the builtin include dirs
        //   might be absolute.

        let include_paths_to_verify = self
            .get_include_dirs()
            .into_iter()
            .chain(self.get_quote_include_dirs())
            .chain(system_include_dirs.iter().cloned());
        for mut include_path in include_paths_to_verify {
            // includePathsToVerify contains all paths that are added as -isystem directive on the
            // command line, most of which are added for include directives in the
            // CcCompilationContext and are thus also in ignoredDirs. The hash lookup prevents
            // this from becoming O(N^2) for these.
            if ignored_dirs.contains(&include_path)
                || starts_with_any(&include_path, &ignored_dirs_list)
            {
                continue;
            }

            // Two conditions:
            // 1. Paths cannot be absolute (e.g. multiple uplevels to /etc/passwd)
            // 2. For relative paths, one starting ../ is okay for getting to a sibling
            //    repository.
            let prefix: &PathFragment = if sibling_repository_layout {
                &EXPERIMENTAL_EXTERNAL_PATH_PREFIX
            } else {
                &EXTERNAL_PATH_PREFIX
            };
            if include_path.starts_with(prefix) {
                include_path = include_path.relative_to(prefix);
            }
            if include_path.is_absolute() || include_path.contains_uplevel_references() {
                let message = format!(
                    "The include path '{}' references a path outside of the execution root.",
                    include_path.get_path_string()
                );
                let code = create_failure_detail(
                    &message,
                    failure_details::cpp_compile::Code::IncludePathOutsideExecRoot,
                );
                return Err(ActionExecutionException::new(
                    message,
                    Some(self),
                    /* catastrophe= */ false,
                    Some(code),
                ));
            }
        }
        Ok(())
    }

    /// `super.updateInputs(inputs)`: `AbstractAction.updateInputs`.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:215 AbstractAction.updateInputs
    fn abstract_update_inputs(&self, inputs: NestedSet<Artifact>) {
        check_state!(
            self.discovers_inputs(),
            "Not an input-discovering action: {}",
            self.to_string_java()
        );
        self.abstract_action().set_inputs(inputs);
        self.set_inputs_discovered(true);
    }

    /// Recalculates this action's live input collection.
    ///
    /// Can only be called if `discoversInputs`, and must be called after execution in that case.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1245 CppCompileAction.updateActionInputs
    pub fn update_action_inputs(&self, discovered_inputs: Option<NestedSet<Artifact>>) {
        check_state!(
            self.discovers_inputs(),
            "Can't call if not discovering inputs: {} {}",
            discovered_inputs
                .as_ref()
                .map_or_else(|| "null".to_owned(), |s| s.to_string()),
            self.to_string_java()
        );
        // PORT: skipped, profiling (ACTION_UPDATE).
        let mut inputs_builder = NestedSetBuilder::<Artifact>::stable_order();
        inputs_builder.add_transitive(&self.mandatory_inputs);
        if let Some(discovered_inputs) = &discovered_inputs {
            inputs_builder.add_transitive(discovered_inputs);
        }
        self.abstract_update_inputs(inputs_builder.build());
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1659 CppCompileAction.getDotDContents
    fn get_dot_d_contents(&self, spawn_result: &SpawnResult) -> Option<Vec<u8>> {
        if let Some(dotd_file) = self.get_dotd_file()
            && let Some(content) = spawn_result.get_in_memory_output(&artifact_input(dotd_file))
        {
            return Some(content.to_vec());
        }
        None
    }

    /// Dynamically compute the dependencies of a compilation using C++20 modules.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1674 CppCompileAction.computeUsedCpp20Modules
    fn compute_used_cpp20_modules(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<Artifact>, ActionExecutionException> {
        if !self
            .feature_configuration
            .is_enabled(cpp_rule_classes::CPP_MODULES)
        {
            return Ok(Vec::new());
        }
        // Module dependency scanning only needs source and header files.
        if self.action_name == CppActionNames::CPP_MODULE_DEPS_SCANNING {
            return Ok(Vec::new());
        }
        if !is_cpp20_module_compilation_action(&self.action_name)
            && !cpp_file_types::CPP_SOURCE.matches_path(self.source_file.get_exec_path())
        {
            return Ok(Vec::new());
        }
        let modmap_input_file = self
            .modmap_input_file
            .as_ref()
            .unwrap_or_else(|| panic!("NullPointerException: modmapInputFile"));
        // Latin-1: the file paths as internal strings.
        let used_module_paths: HashSet<String> = match read_lines_as_latin1(&get_input_path(
            action_execution_context,
            modmap_input_file,
        )) {
            Ok(lines) => lines.into_iter().collect(),
            Err(_) => {
                let message = format!(
                    "failed to read modmap input: {}",
                    modmap_input_file.get_exec_path_string()
                );
                let code = create_failure_detail(
                    &message,
                    failure_details::cpp_compile::Code::ModmapInputFileReadFailure,
                );
                return Err(ActionExecutionException::new(
                    message,
                    Some(self),
                    /* catastrophe= */ false,
                    Some(code),
                ));
            }
        };
        let metadata = blaze_exec::analysis::actions::KeyMetadataAdapter(
            action_execution_context.get_input_metadata_provider(),
        );
        let path_mapper =
            path_mappers::create(self, self.output_paths_mode, false, Some(&metadata));
        // All module files referenced in the modmap input file are expected to be known modules.
        // We delegate error reporting to the compiler by silently skipping over unknown files.
        let module_files = self
            .module_files
            .as_ref()
            .unwrap_or_else(|| panic!("NullPointerException: moduleFiles"));
        Ok(module_files
            .to_list()
            .iter()
            .filter(|module_file| {
                used_module_paths.contains(&path_mapper.get_mapped_exec_path_string(module_file))
            })
            .cloned()
            .collect())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1715 CppCompileAction.createSpawn
    fn create_spawn(
        &self,
        exec_root: &Path,
        client_env: &IndexMap<String, String>,
        path_mapper: &dyn cc::PathMapper,
        // PORT: Java passes one `PathMapper`; `cc::PathMapper` is the C++ crates' view of it and
        // `spawn_path_mapper` the spawn's.
        spawn_path_mapper: SpawnPathMapper,
    ) -> Result<SimpleSpawn, ActionExecutionException> {
        // Intentionally not adding {@link CppCompileAction#inputsForInvalidation}, those are not
        // needed for execution.
        let mut inputs_builder = NestedSetBuilder::<ActionInputRef>::stable_order();
        inputs_builder.add_transitive(&to_action_input_set(&self.mandatory_spawn_inputs));

        if self.discovers_inputs() {
            inputs_builder.add_transitive(&to_action_input_set(&self.get_additional_inputs()));
        }
        if let Some(param_file_action_input) = self.param_file_action_input.lock().clone() {
            inputs_builder.add(ActionInputRef::new(param_file_action_input));
        }
        let inputs = inputs_builder.build();

        let mut execution_info = self.get_execution_info();
        if let Some(dotd_file) = self.get_dotd_file()
            && self.use_in_memory_dotd_files()
        {
            // CppCompileAction does dotd file scanning locally inside the Bazel process and thus
            // requires the dotd file contents to be available locally. In remote execution, we
            // generally don't want to stage all remote outputs on the local file system and thus
            // we need to tell the remote strategy (if any) to at least make the .d file available
            // in-memory. We can do that via ExecutionRequirements.REMOTE_EXECUTION_INLINE_OUTPUTS.
            put_or_throw(
                &mut execution_info,
                REMOTE_EXECUTION_INLINE_OUTPUTS,
                dotd_file.get_exec_path_string().to_owned(),
            );
        }

        if self.should_parse_show_includes() {
            // Hack on Windows. The included headers dumped by cl.exe in stdout contain absolute
            // paths. When compiling the file from different workspace, the shared cache will
            // cause header dependency checking to fail. This was initially fixed by a hack (see
            // https://github.com/bazelbuild/bazel/issues/9172 for more details), but is broken
            // again due to cl/356735700. We require execution service to ignore caches from
            // other workspace.
            put_or_throw(
                &mut execution_info,
                DIFFERENTIATE_WORKSPACE_CACHE,
                exec_root.get_base_name().to_owned(),
            );
        }

        let outputs = self.get_outputs();
        // Without a .gcno file all outputs must be created. In coverage mode, the .gcno file is
        // not produced for an empty translation unit, but the spawn should still succeed.
        let mandatory_outputs: Option<HashSet<ActionInputRef>> =
            self.gcno_file.as_ref().map(|gcno_file| {
                outputs
                    .iter()
                    .filter(|output| *output != gcno_file)
                    .map(artifact_input)
                    .collect()
            });

        let arguments = self
            .get_arguments_with(path_mapper)
            .map_err(|e| self.command_generation_failure(&e.message))?;
        // PORT: the trait's getEffectiveEnvironment uses PathMapper.NOOP; path mapping is OFF
        // (PathMappers.create returns NOOP for every supported mode).
        let environment = self
            .get_effective_environment(client_env)
            .map_err(|e| self.command_generation_failure(&e.message))?;
        Ok(SimpleSpawn::new(
            ActionMetadataSnapshot::of(self),
            arguments,
            environment,
            execution_info,
            inputs,
            /* tools= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            outputs.iter().map(artifact_input).collect(),
            mandatory_outputs,
            Arc::new(CppCompileLocalResources {
                enabled: self.enabled_cpp_compile_resources_estimation(),
                mnemonic: self.get_mnemonic(),
            }),
            spawn_path_mapper,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1839 CppCompileAction.discoverInputsFromDotdFiles
    pub fn discover_inputs_from_dotd_files(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        exec_root: &Path,
        artifact_resolver: &ArtifactFactory,
        dot_d_contents: Option<Vec<u8>>,
        sibling_repository_layout: bool,
        path_mapper: &dyn cc::PathMapper,
    ) -> Result<NestedSet<Artifact>, ActionExecutionException> {
        if self.get_dotd_file().is_none() {
            panic!("NullPointerException: Trying to scan .d file which is unset");
        }
        let dep_set = self.process_depset(action_execution_context, exec_root, dot_d_contents)?;
        let table = CcArtifactTable::default();
        let discovered = header_discovery::discover_inputs_from_dependencies(
            &CppDiscoveringAction {
                action: self,
                table: &table,
            },
            &table.wrap(self.get_source_file()),
            self.needs_include_validation,
            dep_set.get_dependencies(),
            &self.get_permitted_system_include_prefixes(exec_root),
            &table.wrap_set(&self.get_allowed_derived_inputs()),
            exec_root,
            &CcArtifactResolver {
                factory: artifact_resolver,
                table: &table,
            },
            sibling_repository_layout,
            path_mapper,
        )
        .map_err(|e| self.from_cc_exception(e))?;
        Ok(NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            discovered
                .to_list()
                .iter()
                .map(|r| table.unwrap(r))
                .collect::<Vec<_>>(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1861 CppCompileAction.processDepset
    fn process_depset(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
        exec_root: &Path,
        dot_d_contents: Option<Vec<u8>>,
    ) -> Result<DependencySet, ActionExecutionException> {
        let mut dep_set = DependencySet::new(exec_root.clone());
        let result = match dot_d_contents {
            Some(contents) if self.cpp_configuration().get_inmemory_dotd_files() => {
                dep_set.process(contents).map(|_| ())
            }
            _ => {
                let dotd_file = self
                    .get_dotd_file()
                    .unwrap_or_else(|| panic!("NullPointerException: dotdFile"));
                dep_set
                    .read(&get_input_path(action_execution_context, dotd_file))
                    .map(|_| ())
            }
        };
        match result {
            Ok(()) => Ok(dep_set),
            Err(e) => {
                // Some kind of IO or parse exception--wrap & rethrow it to stop the build.
                let message = format!("error while parsing .d file: {e}");
                let code = create_failure_detail(
                    &message,
                    failure_details::cpp_compile::Code::DFileParseFailure,
                );
                Err(ActionExecutionException::with_cause(
                    message,
                    &e,
                    Some(self),
                    false,
                    Some(code),
                ))
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1878 CppCompileAction.getPermittedSystemIncludePrefixes
    fn get_permitted_system_include_prefixes(&self, exec_root: &Path) -> Vec<Path> {
        let mut system_include_prefixes = Vec::new();
        for include_path in self.get_built_in_include_directories() {
            if include_path.is_absolute() {
                system_include_prefixes.push(Path::create(
                    include_path.clone(),
                    exec_root.get_file_system(),
                ));
            }
        }
        system_include_prefixes
    }

    /// Gcc only creates a ".gcno" file if the compilation unit is non-empty. To ensure that the
    /// set of outputs for a CppCompileAction remains consistent and doesn't vary dynamically
    /// depending on the _contents_ of the input files, we create an empty ".gcno" file if gcc
    /// didn't create it.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1893 CppCompileAction.ensureCoverageNotesFileExists
    fn ensure_coverage_notes_file_exists(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<(), ActionExecutionException> {
        let Some(gcno_file) = &self.gcno_file else {
            return Ok(());
        };
        if !cpp_file_types::COVERAGE_NOTES.matches_artifact(gcno_file) {
            // PORT: BugReport.sendNonFatalBugReport is not ported; the non-fatal report is
            // printed to stderr.
            system_err_println(&format!(
                "java.lang.IllegalStateException: In coverage mode but gcno artifact is not \
                 correct type: {}, {}",
                gcno_file,
                self.to_string_java()
            ));
            return Ok(());
        }
        let output_path = get_input_path(action_execution_context, gcno_file);
        if output_path.exists() {
            return Ok(());
        }
        if let Err(e) =
            blaze_exec::analysis::actions::file_system_utils::create_empty_file(&output_path)
        {
            let message = format!(
                "Error creating file '{}': {}",
                output_path.get_path_string(),
                e
            );
            let code = create_failure_detail(
                &message,
                failure_details::cpp_compile::Code::CoverageNotesCreationFailure,
            );
            return Err(ActionExecutionException::with_cause(
                message,
                &e,
                Some(self),
                false,
                Some(code),
            ));
        }
        Ok(())
    }
}

#[allow(clippy::result_large_err)]
impl ExecutableAction for CppCompileAction {
    /// This method returns null when a required SkyValue is missing and a Skyframe restart is
    /// required.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:663 CppCompileAction.discoverInputs
    fn discover_inputs(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<NestedSet<Artifact>>, ActionExecutionException> {
        check_argument!(
            !cpp_file_types::CPP_MODULE.matches_artifact(&self.source_file)
                || is_cpp20_module_compilation_action(&self.action_name)
        );

        let additional_inputs_unset = self.additional_inputs.lock().is_none();
        if additional_inputs_unset {
            let options = match self
                .compile_command_line
                .get_compiler_options(None, &CC_NOOP)
            {
                Ok(options) => options,
                Err(e) => return Err(self.command_generation_failure(&e.message)),
            };
            // commandLineKey = computeCommandLineKey(options);
            // PORT: the key is kept in a OnceLock; Java recomputes the same value here.
            let _ = self
                .command_line_key
                .get_or_init(|| compute_command_line_key(&options));
            let system_include_dirs = self.get_system_include_dirs_from(&options);
            let sibling_layout = experimental_sibling_repository_layout(action_execution_context);
            if !self.should_scan_includes {
                let used_cpp20_modules =
                    self.compute_used_cpp20_modules(action_execution_context)?;
                // When not actually doing include scanning, add all prunable headers to
                // additionalInputs. This is necessary because the inputs that can be pruned by .d
                // file parsing must be returned from discoverInputs() and they cannot be in
                // mandatoryInputs. Thus, even with include scanning turned off, we pretend that we
                // "discover" these headers.
                let mut builder = NestedSetBuilder::from_nested_set(
                    self.cc_compilation_context.get_declared_include_srcs(),
                );
                builder
                    .add_transitive(&self.additional_prunable_headers)
                    .add_all(used_cpp20_modules.iter().cloned());
                let additional_inputs = builder.build();
                *self.used_cpp20_modules.lock() = Some(used_cpp20_modules);
                *self.additional_inputs.lock() = Some(additional_inputs.clone());
                if self.needs_include_validation {
                    self.verify_action_include_paths(&system_include_dirs, sibling_layout)?;
                }
                return Ok(Some(additional_inputs));
            }
            blaze_util::unported!(
                "CppCompileAction.discoverInputs",
                "include scanning (IncludeScanningHeaderData, CppIncludeScanningContext.findAdditionalInputs)"
            );
        }

        // usedModules == null: there are two paths in which this can be reached:
        // 1. This is not a modular compilation or one without include scanning. In either case,
        //    we never compute used modules.
        // 2. This function has completed on a previous execution, adding all used modules to
        //    additionalInputs and resetting usedModules to null below.
        // In either case, there is nothing more to do here.
        // PORT: usedModules is only set by the include-scanning path (unported above).
        Ok(self.additional_inputs.lock().clone())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1484 CppCompileAction.execute
    fn execute(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let metadata = blaze_exec::analysis::actions::KeyMetadataAdapter(
            action_execution_context.get_input_metadata_provider(),
        );
        let path_mapper_ref =
            path_mappers::create(self, self.output_paths_mode, false, Some(&metadata));
        let path_mapper = CcPathMapper(&*path_mapper_ref);

        if self
            .feature_configuration
            .is_enabled(cpp_rule_classes::COMPILER_PARAM_FILE)
        {
            let overwritten = self.get_overwritten_variables();
            match self
                .compile_command_line
                .get_compiler_options(Some(&*overwritten), &path_mapper)
            {
                Ok(compiler_options) => {
                    *self.param_file_action_input.lock() = Some(ParamFileActionInput::new(
                        self.param_file_path
                            .clone()
                            .unwrap_or_else(|| panic!("NullPointerException: paramFilePath")),
                        compiler_options,
                        // TODO(b/132888308): Support MSVC, which has its own method of escaping
                        // strings.
                        ParameterFileType::GCC_QUOTED,
                    ));
                }
                Err(e) => return Err(self.command_generation_failure(&e.message)),
            }
        }

        if self.should_scan_includes {
            let additional_inputs = self.additional_inputs.lock().clone();
            self.update_action_inputs(additional_inputs);
        }

        if self.should_parse_show_includes() {
            // PORT: /showIncludes parsing (ShowIncludesFilter over a child FileOutErr) is used
            // with MSVC only.
            blaze_util::unported!(
                "CppCompileAction.execute",
                "parse_showincludes (ShowIncludesFilter, FileOutErr.childOutErr)"
            );
        }

        let spawn = self.create_spawn(
            action_execution_context.get_exec_root(),
            action_execution_context.get_client_env(),
            &path_mapper,
            SpawnPathMapper::of(path_mapper_ref.clone()),
        );
        // finally
        self.clear_additional_inputs();
        let spawn = spawn?;

        let resolver = get_context::<SpawnStrategyResolver>(action_execution_context)
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: getContext(SpawnStrategyResolver.class) returned null"
                )
            });
        let spawn_results = resolver
            .exec(&spawn, action_execution_context)
            .map_err(|e| ActionExecutionException::from_exec_exception(&e, None, self))?;
        // SpawnActionContext guarantees that the first list entry exists and corresponds to the
        // executed spawn.
        let dot_d_contents = self.get_dot_d_contents(&spawn_results[0]);
        // copyTempOutErrToActionOutErrMaybe: a no-op unless shouldParseShowIncludes().

        self.ensure_coverage_notes_file_exists(action_execution_context)?;

        let scanning_context = get_cpp_include_extraction_context(action_execution_context);
        let exec_root = action_execution_context.get_exec_root();
        let sibling_repository_layout =
            experimental_sibling_repository_layout(action_execution_context);

        if self.get_dotd_file().is_none() {
            return Ok(ActionResult::create(spawn_results));
        }

        // Post-execute "include scanning", which modifies the action inputs to match what the
        // compile action actually used by incorporating the results of .d file parsing.
        let scanning_context = scanning_context.unwrap_or_else(|| {
            panic!(
                "NullPointerException: getContext(CppIncludeExtractionContext.class) returned null"
            )
        });
        let mut discovered_inputs = self.discover_inputs_from_dotd_files(
            action_execution_context,
            exec_root,
            scanning_context.get_artifact_resolver(),
            dot_d_contents,
            sibling_repository_layout,
            &path_mapper,
        )?;

        let used_cpp20_modules = self.used_cpp20_modules.lock().clone();
        if let Some(used_cpp20_modules) = used_cpp20_modules {
            let mut builder = NestedSetBuilder::<Artifact>::stable_order();
            builder
                .add_all(used_cpp20_modules)
                .add_transitive(&discovered_inputs);
            discovered_inputs = builder.build();
        }
        self.update_action_inputs(Some(discovered_inputs.clone()));

        // hdrs_check: This cannot be switched off for C++ build actions,
        // because doing so would allow for incorrect builds.
        // HeadersCheckingMode.NONE should only be used for ObjC build actions.
        self.validate_inclusions(&discovered_inputs)?;
        Ok(ActionResult::create(spawn_results))
    }
}

// -------------------------------------------------------------------------------------------
// CppCompileActionBuilder

/// Exception thrown when the action is not configured in the toolchain.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:246 CppCompileActionBuilder.UnconfiguredActionConfigException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnconfiguredActionConfigException {
    pub message: String,
}

/// Errors of `buildAndVerify`.
#[derive(Debug)]
pub enum BuildAndVerifyError {
    UnconfiguredActionConfig(UnconfiguredActionConfigException),
    Eval(starlark::Error),
}

/// What the builder reads from `CcToolchainProvider`, read when the builder is created (the
/// provider is a Starlark value on the target's heap). Each getter's `EvalException` message is
/// kept and raised where Java calls the getter.
#[derive(Debug, Clone)]
pub struct CcToolchainCompileData {
    pub compiler_files: Result<NestedSet<Artifact>, String>,
    pub compiler_files_without_includes: Result<NestedSet<Artifact>, String>,
    pub as_files: Result<NestedSet<Artifact>, String>,
    pub all_files: Result<NestedSet<Artifact>, String>,
    pub builtin_include_files: Result<Vec<Artifact>, String>,
    pub built_in_include_directories: Result<Vec<PathFragment>, String>,
    pub grep_includes: Result<Option<Artifact>, String>,
}

fn eval_err<T: Clone>(r: &Result<T, String>) -> Result<T, BuildAndVerifyError> {
    r.clone()
        .map_err(|e| BuildAndVerifyError::Eval(super::util::errorf(e)))
}

/// Builder class to construct C++ compile actions.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:45 CppCompileActionBuilder
pub struct CppCompileActionBuilder {
    owner: ActionOwner,
    shareable: bool,
    configuration: Arc<BuildConfigurationValue>,
    feature_configuration: Option<Arc<FeatureConfiguration>>,
    variables: CcToolchainVariablesRef,
    source_file: Option<Artifact>,
    mandatory_inputs_builder: NestedSetBuilder<Artifact>,
    output_file: Option<Artifact>,
    modmap_file: Option<Artifact>,
    modmap_input_file: Option<Artifact>,
    module_files: Option<NestedSet<Artifact>>,
    dwo_file: Option<Artifact>,
    lto_indexing_file: Option<Artifact>,
    dotd_file: Option<Artifact>,
    diagnostics_file: Option<Artifact>,
    gcno_file: Option<Artifact>,
    cc_compilation_context: Option<CcCompilationContext>,
    copts_filter: CoptsFilter,
    use_pic: bool,
    additional_include_scanning_roots: Vec<Artifact>,
    should_scan_includes: Option<bool>,
    execution_info: IndexMap<String, String>,
    cc_toolchain: CcToolchainCompileData,
    action_name: Option<String>,
    build_info_header_artifacts: Vec<Artifact>,
    cache_key_inputs: NestedSet<Artifact>,
    additional_prunable_headers: NestedSet<Artifact>,
    additional_outputs: Vec<Artifact>,
    needs_include_validation: bool,
    /// The execution platform of `owner` (Java `owner.getExecutionPlatform()`), for
    /// `CppCompileAction.getExecutionOs`.
    execution_platform: Option<Arc<AnalysisPlatformInfo>>,
}

impl CppCompileActionBuilder {
    /// Creates a builder from a rule and configuration. `owner` is the action owner the action
    /// construction context gives (Java computes it from the context and the toolchain type;
    /// see `cc_starlark_internal`).
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:84 CppCompileActionBuilder.<init>
    pub fn new(
        owner: ActionOwner,
        cc_toolchain: CcToolchainCompileData,
        configuration: Arc<BuildConfigurationValue>,
    ) -> CppCompileActionBuilder {
        CppCompileActionBuilder {
            owner,
            shareable: false,
            configuration,
            feature_configuration: None,
            variables: CcToolchainVariables::empty(),
            source_file: None,
            mandatory_inputs_builder: NestedSetBuilder::stable_order(),
            output_file: None,
            modmap_file: None,
            modmap_input_file: None,
            module_files: None,
            dwo_file: None,
            lto_indexing_file: None,
            dotd_file: None,
            diagnostics_file: None,
            gcno_file: None,
            cc_compilation_context: None,
            copts_filter: CoptsFilter::always_passes(),
            use_pic: false,
            additional_include_scanning_roots: Vec::new(),
            should_scan_includes: None,
            execution_info: IndexMap::new(),
            cc_toolchain,
            action_name: None,
            build_info_header_artifacts: Vec::new(),
            cache_key_inputs: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            additional_prunable_headers: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            additional_outputs: Vec::new(),
            needs_include_validation: false,
            execution_platform: None,
        }
    }

    /// Creates a builder that is a copy of another builder.
    ///
    /// PORT: Java's copy constructor leaves `variables`, `diagnosticsFile`,
    /// `buildInfoHeaderArtifacts`, `cacheKeyInputs` and `additionalPrunableHeaders` at their
    /// field defaults; `pluginOpts` and `extraSystemIncludePrefixes` have no port (never set by
    /// the Starlark natives). The Rust-only `execution_platform` (the owner's) is copied with the
    /// owner.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:110 CppCompileActionBuilder.<init>
    pub fn copy_of(other: &CppCompileActionBuilder) -> CppCompileActionBuilder {
        let mut mandatory_inputs_builder = NestedSetBuilder::stable_order();
        mandatory_inputs_builder.add_transitive(&other.mandatory_inputs_builder.build());
        CppCompileActionBuilder {
            owner: other.owner.clone(),
            shareable: other.shareable,
            configuration: other.configuration.clone(),
            feature_configuration: other.feature_configuration.clone(),
            variables: CcToolchainVariables::empty(),
            source_file: other.source_file.clone(),
            mandatory_inputs_builder,
            output_file: other.output_file.clone(),
            modmap_file: other.modmap_file.clone(),
            modmap_input_file: other.modmap_input_file.clone(),
            module_files: other.module_files.clone(),
            dwo_file: other.dwo_file.clone(),
            lto_indexing_file: other.lto_indexing_file.clone(),
            dotd_file: other.dotd_file.clone(),
            diagnostics_file: None,
            gcno_file: other.gcno_file.clone(),
            cc_compilation_context: other.cc_compilation_context.clone(),
            copts_filter: other.copts_filter.clone(),
            use_pic: other.use_pic,
            additional_include_scanning_roots: other.additional_include_scanning_roots.clone(),
            should_scan_includes: other.should_scan_includes,
            execution_info: other.execution_info.clone(),
            cc_toolchain: other.cc_toolchain.clone(),
            action_name: other.action_name.clone(),
            build_info_header_artifacts: Vec::new(),
            cache_key_inputs: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            additional_prunable_headers: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            additional_outputs: other.additional_outputs.clone(),
            needs_include_validation: other.needs_include_validation,
            execution_platform: other.execution_platform.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:180 CppCompileActionBuilder.getOwner
    pub fn get_owner(&self) -> &ActionOwner {
        &self.owner
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:188 CppCompileActionBuilder.getCcCompilationContext
    pub fn get_cc_compilation_context(&self) -> &CcCompilationContext {
        self.cc_compilation_context
            .as_ref()
            .expect("NullPointerException: ccCompilationContext")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:401 CppCompileActionBuilder.getPrunableHeaders
    pub fn get_prunable_headers(&self) -> &NestedSet<Artifact> {
        &self.additional_prunable_headers
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:405 CppCompileActionBuilder.getInputsForInvalidation
    pub fn get_inputs_for_invalidation(&self) -> &NestedSet<Artifact> {
        self.get_cc_compilation_context()
            .get_declared_include_srcs()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:458 CppCompileActionBuilder.getFeatureConfiguration
    pub fn get_feature_configuration(&self) -> &Arc<FeatureConfiguration> {
        self.feature_configuration()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:470 CppCompileActionBuilder.getVariables
    pub fn get_variables(&self) -> &CcToolchainVariablesRef {
        &self.variables
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:480 CppCompileActionBuilder.getExecutionInfo
    pub fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:509 CppCompileActionBuilder.useDotdFile
    pub fn use_dotd_file(&self, source_file: &Artifact) -> bool {
        cpp_file_types::header_discovery_required(source_file)
            && !self.use_header_modules_for(source_file)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:610 CppCompileActionBuilder.getCoptsFilter
    pub fn get_copts_filter(&self) -> &CoptsFilter {
        &self.copts_filter
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:627 CppCompileActionBuilder.getActionEnvironment
    pub fn get_action_environment(&self) -> Arc<ActionEnvironment> {
        action_environment(&self.configuration)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:656 CppCompileActionBuilder.getBuiltinIncludeDirectories
    pub fn get_builtin_include_directories(&self) -> Result<Vec<PathFragment>, String> {
        self.cc_toolchain.built_in_include_directories.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:660 CppCompileActionBuilder.shouldCompileHeaders
    pub fn should_compile_headers(&self) -> bool {
        // CcToolchainProvider.shouldProcessHeaders(featureConfiguration, cppConfiguration)
        // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:239 CcToolchainProvider.shouldProcessHeaders
        self.feature_configuration()
            .is_enabled(cpp_rule_classes::PARSE_HEADERS)
    }

    /// `buildMandatoryInputs()` for `CppCompileActionTemplate` (package-private in Java).
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:371 CppCompileActionBuilder.buildMandatoryInputs
    pub fn build_mandatory_inputs_for_template(
        &mut self,
    ) -> Result<NestedSet<Artifact>, BuildAndVerifyError> {
        self.build_mandatory_inputs()
    }

    /// Records the execution platform of the builder's action owner (Java reads it from the
    /// owner: `ActionOwner.getExecutionPlatform()`).
    pub fn set_execution_platform(
        &mut self,
        execution_platform: Option<Arc<AnalysisPlatformInfo>>,
    ) -> &mut Self {
        self.execution_platform = execution_platform;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:143 CppCompileActionBuilder.setSourceFile
    pub fn set_source_file(&mut self, source_file: Artifact) -> &mut Self {
        check_state!(
            self.source_file.is_none(),
            "New source file {} trying to overwrite old source file {}",
            source_file,
            self.source_file
                .as_ref()
                .map_or("null".to_owned(), |a| a.to_string())
        );
        self.source_file = Some(source_file);
        self
    }

    /// The `setSourceFile(TreeFileArtifact)` overload: a tree file artifact may replace a source
    /// that is not one (the template's builder carries the source tree artifact).
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:153 CppCompileActionBuilder.setSourceFile
    pub fn set_source_file_tree_file(&mut self, source_file: Artifact) -> &mut Self {
        check_state!(
            self.source_file.as_ref().is_none_or(|a| a.artifact_class()
                != blaze_actions::actions::artifact::ArtifactClass::TreeFileArtifact),
            "New source file {} trying to overwrite old source file {} also a tree file artifact",
            source_file,
            self.source_file
                .as_ref()
                .map_or("null".to_owned(), |a| a.to_string())
        );
        self.source_file = Some(source_file);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:169 CppCompileActionBuilder.setAdditionalOutputs
    pub fn set_additional_outputs(&mut self, additional_outputs: Vec<Artifact>) -> &mut Self {
        self.additional_outputs = additional_outputs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:175 CppCompileActionBuilder.setNeedsIncludeValidation
    pub fn set_needs_include_validation(&mut self, needs_include_validation: bool) -> &mut Self {
        self.needs_include_validation = needs_include_validation;
        self
    }

    fn source_file(&self) -> &Artifact {
        self.source_file
            .as_ref()
            .expect("NullPointerException: sourceFile")
    }

    fn feature_configuration(&self) -> &Arc<FeatureConfiguration> {
        self.feature_configuration
            .as_ref()
            .expect("NullPointerException: featureConfiguration")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:192 CppCompileActionBuilder.getActionName
    pub fn get_action_name(&self) -> String {
        if let Some(action_name) = &self.action_name {
            return action_name.clone();
        }
        let source_path = self.source_file().get_exec_path();
        if cpp_file_types::CPP_MODULE_MAP.matches_path(source_path) {
            CppActionNames::CPP_MODULE_COMPILE.to_owned()
        } else if cpp_file_types::CPP_HEADER.matches_path(source_path) {
            // TODO(bazel-team): Handle C headers that probably don't work in C++ mode.
            if self
                .feature_configuration()
                .is_enabled(cpp_rule_classes::PARSE_HEADERS)
            {
                return CppActionNames::CPP_HEADER_PARSING.to_owned();
            }
            // CcCommon.collectCAndCppSources() ensures we do not add headers to
            // the compilation artifacts unless 'parse_headers' is set.
            panic!("IllegalStateException");
        } else if cpp_file_types::C_SOURCE.matches_path(source_path) {
            CppActionNames::C_COMPILE.to_owned()
        } else if cpp_file_types::CPP_SOURCE.matches_path(source_path) {
            CppActionNames::CPP_COMPILE.to_owned()
        } else if cpp_file_types::OBJC_SOURCE.matches_path(source_path) {
            CppActionNames::OBJC_COMPILE.to_owned()
        } else if cpp_file_types::OBJCPP_SOURCE.matches_path(source_path) {
            CppActionNames::OBJCPP_COMPILE.to_owned()
        } else if cpp_file_types::ASSEMBLER.matches_path(source_path) {
            CppActionNames::ASSEMBLE.to_owned()
        } else if cpp_file_types::ASSEMBLER_WITH_C_PREPROCESSOR.matches_path(source_path) {
            CppActionNames::PREPROCESS_ASSEMBLE.to_owned()
        } else if cpp_file_types::CLIF_INPUT_PROTO.matches_path(source_path) {
            CppActionNames::CLIF_MATCH.to_owned()
        } else if cpp_file_types::CPP_MODULE.matches_path(source_path) {
            CppActionNames::CPP_MODULE_CODEGEN.to_owned()
        } else {
            // CcCompilationHelper ensures CppCompileAction only gets instantiated for supported
            // file types.
            panic!("IllegalStateException");
        }
    }

    /// Builds the Action as configured and performs some validations on the action.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:239 CppCompileActionBuilder.buildAndVerify
    pub fn build_and_verify(&mut self) -> Result<CppCompileAction, BuildAndVerifyError> {
        let feature_configuration = self.feature_configuration().clone();
        let use_header_modules = self.use_header_modules();

        let action_name = self.get_action_name();
        if feature_configuration.action_is_configured(&action_name) {
            for execution_requirement in feature_configuration
                .get_tool_requirements_for_action(&action_name)
                .iter()
            {
                self.execution_info
                    .insert(execution_requirement.clone(), String::new());
            }
        } else {
            return Err(BuildAndVerifyError::UnconfiguredActionConfig(
                UnconfiguredActionConfigException {
                    message: format!("Expected action_config for '{action_name}' to be configured"),
                },
            ));
        }

        // If include scanning is enabled, we can use the filegroup without header files - they
        // are found by include scanning. We still need the system framework headers since they
        // are not being scanned right now, but those are placed in the "compile" file group.
        let mut compiler_files_without_includes =
            eval_err(&self.cc_toolchain.compiler_files_without_includes)?;
        if compiler_files_without_includes.is_empty() {
            compiler_files_without_includes = eval_err(&self.cc_toolchain.compiler_files)?;
        }
        let should_scan_includes = self.get_should_scan_includes();
        let toolchain_inputs = if should_scan_includes {
            compiler_files_without_includes
        } else if cpp_configuration(&self.configuration).use_specific_tool_files()
            && !self.source_file().is_tree_artifact()
        {
            if self.get_action_name() == CppActionNames::ASSEMBLE {
                eval_err(&self.cc_toolchain.as_files)?
            } else {
                eval_err(&self.cc_toolchain.compiler_files)?
            }
        } else {
            eval_err(&self.cc_toolchain.all_files)?
        };
        self.add_transitive_mandatory_inputs(&toolchain_inputs);

        let real_mandatory_spawn_inputs = self.build_mandatory_inputs()?;
        let mut b = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        b.add_transitive(&real_mandatory_spawn_inputs)
            .add_transitive(&self.cache_key_inputs);
        let real_mandatory_inputs = b.build();
        let mut prunable_headers = self.additional_prunable_headers.clone();
        if should_scan_includes {
            // With include scanning enabled, only compiler_files_without_includes is staged as a
            // mandatory input; the rest of the toolchain files (compiler_files) are expected to
            // be discovered on demand by the include scanner.
            let mut p = NestedSetBuilder::stable_order();
            p.add_transitive(&self.additional_prunable_headers)
                .add_transitive(&eval_err(&self.cc_toolchain.compiler_files)?);
            prunable_headers = p.build();
        }

        let mnemonic = action_name_to_mnemonic(
            &action_name,
            &feature_configuration,
            cpp_configuration(&self.configuration).use_cpp_compile_header_mnemonic(),
        );
        blaze_analysis::rule::config_util::modify_execution_info(
            &self.configuration,
            &mut self.execution_info,
            &mnemonic,
        );

        let builtin_include_files = self.get_builtin_include_files()?;
        let built_in_include_directories =
            eval_err(&self.cc_toolchain.built_in_include_directories)?;
        let grep_includes = eval_err(&self.cc_toolchain.grep_includes)?;
        // Copying the collections is needed to make the builder reusable.
        let mut action = CppCompileAction::new(
            self.owner.clone(),
            feature_configuration,
            self.variables.clone(),
            self.source_file().clone(),
            self.configuration.clone(),
            self.shareable,
            should_scan_includes,
            self.use_pic,
            use_header_modules,
            real_mandatory_inputs,
            real_mandatory_spawn_inputs,
            builtin_include_files,
            prunable_headers,
            self.output_file
                .clone()
                .expect("NullPointerException: outputFile"),
            self.dotd_file.clone(),
            self.diagnostics_file.clone(),
            self.gcno_file.clone(),
            self.dwo_file.clone(),
            self.lto_indexing_file.clone(),
            self.cc_compilation_context
                .clone()
                .expect("NullPointerException: ccCompilationContext"),
            self.copts_filter.clone(),
            self.additional_include_scanning_roots.clone(),
            self.execution_info.clone(),
            action_name,
            self.needs_include_validation,
            built_in_include_directories,
            grep_includes,
            self.additional_outputs.clone(),
            self.module_files.clone(),
            self.modmap_input_file.clone(),
        );
        action.execution_platform = self.execution_platform.clone();
        Ok(action)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:327 CppCompileActionBuilder.getBuiltinIncludeFiles
    fn get_builtin_include_files(&self) -> Result<Vec<Artifact>, BuildAndVerifyError> {
        let builtin_include_files = eval_err(&self.cc_toolchain.builtin_include_files)?;
        if self.build_info_header_artifacts.is_empty() {
            return Ok(builtin_include_files);
        }
        if builtin_include_files.is_empty() {
            return Ok(self.build_info_header_artifacts.clone());
        }
        let mut v = builtin_include_files;
        v.extend(self.build_info_header_artifacts.iter().cloned());
        Ok(v)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:366 CppCompileActionBuilder.shouldParseShowIncludes
    fn should_parse_show_includes(&self) -> bool {
        self.feature_configuration()
            .is_enabled(cpp_rule_classes::PARSE_SHOWINCLUDES)
    }

    /// Returns the list of mandatory inputs for the `CppCompileAction` as configured.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:371 CppCompileActionBuilder.buildMandatoryInputs
    fn build_mandatory_inputs(&mut self) -> Result<NestedSet<Artifact>, BuildAndVerifyError> {
        let cc_compilation_context = self
            .cc_compilation_context
            .clone()
            .expect("NullPointerException: ccCompilationContext");
        let mut real_mandatory_inputs_builder = NestedSetBuilder::compile_order();
        real_mandatory_inputs_builder.add_transitive(&self.mandatory_inputs_builder.build());
        real_mandatory_inputs_builder.add_all(self.get_builtin_include_files()?);
        let should_scan_includes = self.get_should_scan_includes();
        if (self.use_header_modules() || self.load_header_modules()) && !should_scan_includes {
            real_mandatory_inputs_builder
                .add_transitive(cc_compilation_context.get_transitive_modules(self.use_pic));
            // The separate module of this compilation context is not part of the transitive
            // modules, but may be used by all compiles of this context except for its own
            // compile; see CcCompilationContext#getDirectModules.
            if let Some(separate_module) =
                cc_compilation_context.get_separate_header_module(self.use_pic)
            {
                if Some(separate_module) != self.output_file.as_ref() {
                    real_mandatory_inputs_builder.add(separate_module.clone());
                }
            }
        }
        cc_compilation_context.add_additional_inputs(&mut real_mandatory_inputs_builder);
        real_mandatory_inputs_builder.add(self.source_file().clone());
        if let Some(grep_includes) = eval_err(&self.cc_toolchain.grep_includes)? {
            real_mandatory_inputs_builder.add(grep_includes);
        }
        if !should_scan_includes && self.dotd_file.is_none() && !self.should_parse_show_includes() {
            real_mandatory_inputs_builder
                .add_transitive(cc_compilation_context.get_declared_include_srcs());
            real_mandatory_inputs_builder.add_transitive(&self.additional_prunable_headers);
        }
        if let Some(modmap_file) = &self.modmap_file {
            real_mandatory_inputs_builder.add(modmap_file.clone()).add(
                self.modmap_input_file
                    .clone()
                    .expect("NullPointerException: modmapInputFile"),
            );
        }
        Ok(real_mandatory_inputs_builder.build())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:409 CppCompileActionBuilder.useHeaderModules
    fn use_header_modules_for(&self, source_file: &Artifact) -> bool {
        self.feature_configuration()
            .is_enabled(cpp_rule_classes::USE_HEADER_MODULES)
            && (cpp_file_types::CPP_SOURCE.matches_artifact(source_file)
                || cpp_file_types::CPP_HEADER.matches_artifact(source_file)
                || cpp_file_types::CPP_MODULE_MAP.matches_artifact(source_file))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:409 CppCompileActionBuilder.useHeaderModules
    fn use_header_modules(&self) -> bool {
        self.use_header_modules_for(self.source_file())
    }

    /// Whether this is a module codegen action that may load (but doesn't compile against)
    /// transitive modules.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:426 CppCompileActionBuilder.loadHeaderModules
    fn load_header_modules(&self) -> bool {
        // The module file imports the modules of dependencies iff it was built with
        // USE_HEADER_MODULES.
        self.feature_configuration()
            .is_enabled(cpp_rule_classes::USE_HEADER_MODULES)
            && cpp_file_types::CPP_MODULE.matches_artifact(self.source_file())
    }

    /// Set action name that is used to pick the right action_config and features from
    /// `FeatureConfiguration`. By default the action name is decided from the source filetype.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:439 CppCompileActionBuilder.setActionName
    pub fn set_action_name(&mut self, action_name: &str) -> &mut Self {
        check_state!(
            self.action_name.is_none(),
            "New actionName {} trying to overwrite old name {}",
            action_name,
            self.action_name.as_deref().unwrap_or("null")
        );
        self.action_name = Some(action_name.to_owned());
        self
    }

    /// Sets the feature configuration to be used for the action.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:451 CppCompileActionBuilder.setFeatureConfiguration
    pub fn set_feature_configuration(
        &mut self,
        feature_configuration: Arc<FeatureConfiguration>,
    ) -> &mut Self {
        self.feature_configuration = Some(feature_configuration);
        self
    }

    /// Sets the feature build variables to be used for the action.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:464 CppCompileActionBuilder.setVariables
    pub fn set_variables(&mut self, variables: CcToolchainVariablesRef) -> &mut Self {
        self.variables = variables;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:475 CppCompileActionBuilder.addExecutionInfo
    pub fn add_execution_info(&mut self, execution_info: &IndexMap<String, String>) -> &mut Self {
        for (k, v) in execution_info {
            self.execution_info.insert(k.clone(), v.clone());
        }
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:485 CppCompileActionBuilder.addMandatoryInputs
    pub fn add_mandatory_inputs_set(&mut self, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.mandatory_inputs_builder.add_transitive(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:485 CppCompileActionBuilder.addMandatoryInputs
    pub fn add_mandatory_inputs(&mut self, artifacts: Vec<Artifact>) -> &mut Self {
        self.mandatory_inputs_builder.add_all(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:497 CppCompileActionBuilder.addTransitiveMandatoryInputs
    pub fn add_transitive_mandatory_inputs(
        &mut self,
        artifacts: &NestedSet<Artifact>,
    ) -> &mut Self {
        self.mandatory_inputs_builder.add_transitive(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:503 CppCompileActionBuilder.addAdditionalIncludeScanningRoots
    pub fn add_additional_include_scanning_roots(&mut self, roots: Vec<Artifact>) -> &mut Self {
        self.additional_include_scanning_roots.extend(roots);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:514 CppCompileActionBuilder.setOutputs
    pub fn set_outputs(
        &mut self,
        output_file: Option<Artifact>,
        dotd_file: Option<Artifact>,
        diagnostics_file: Option<Artifact>,
    ) -> &mut Self {
        self.output_file = output_file;
        self.dotd_file = dotd_file;
        self.diagnostics_file = diagnostics_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:523 CppCompileActionBuilder.setDwoFile
    pub fn set_dwo_file(&mut self, dwo_file: Option<Artifact>) -> &mut Self {
        self.dwo_file = dwo_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:533 CppCompileActionBuilder.setLtoIndexingFile
    pub fn set_lto_indexing_file(&mut self, lto_indexing_file: Option<Artifact>) -> &mut Self {
        self.lto_indexing_file = lto_indexing_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:539 CppCompileActionBuilder.setGcnoFile
    pub fn set_gcno_file(&mut self, gcno_file: Option<Artifact>) -> &mut Self {
        self.gcno_file = gcno_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:545 CppCompileActionBuilder.setCcCompilationContext
    pub fn set_cc_compilation_context(
        &mut self,
        cc_compilation_context: CcCompilationContext,
    ) -> &mut Self {
        self.cc_compilation_context = Some(cc_compilation_context);
        self
    }

    /// Sets whether the CompileAction should use pic mode.
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:553 CppCompileActionBuilder.setPicMode
    pub fn set_pic_mode(&mut self, use_pic: bool) -> &mut Self {
        self.use_pic = use_pic;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:559 CppCompileActionBuilder.setShareable
    pub fn set_shareable(&mut self, shareable: bool) -> &mut Self {
        self.shareable = shareable;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:565 CppCompileActionBuilder.setShouldScanIncludes
    pub fn set_should_scan_includes(&mut self, should_scan_includes: bool) -> &mut Self {
        self.should_scan_includes = Some(should_scan_includes);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:570 CppCompileActionBuilder.getShouldScanIncludes
    pub fn get_should_scan_includes(&mut self) -> bool {
        if let Some(should_scan_includes) = self.should_scan_includes {
            return should_scan_includes;
        }

        let module_deps_scanning_action =
            self.action_name.as_deref() == Some(CppActionNames::CPP_MODULE_DEPS_SCANNING);

        // Three things have to be true to perform #include scanning:
        //  1. The toolchain configuration has to generally enable it.
        //     This is the default unless the --nocc_include_scanning flag was specified.
        //  2. The action is not CPP_MODULE_DEPS_SCANNING.
        //  3. The rule or package must not disable it via 'features = ["-cc_include_scanning"]'.
        //     Normally the scanner is enabled, but rules with precisely specified
        //     dependencies not understood by the scanner can selectively disable it.
        //  4. The file must not be a not-for-preprocessing assembler source file.
        //     Assembler without C preprocessing can use the '.include' pseudo-op which is not
        //     understood by the include scanner, so we'll disable scanning, and instead require
        //     the declared sources to state (possibly overapproximate) the dependencies.
        //     Assembler with preprocessing can also use '.include', but supporting both kinds
        //     of inclusion for that use-case is ridiculous.
        let should_scan_includes = cpp_configuration(&self.configuration).should_scan_includes()
            && self
                .feature_configuration()
                .get_requested_features()
                .contains("cc_include_scanning")
            && !module_deps_scanning_action
            && !cpp_file_types::ASSEMBLER.matches_artifact(self.source_file())
            && !cpp_file_types::CPP_MODULE.matches_artifact(self.source_file());
        self.should_scan_includes = Some(should_scan_includes);
        should_scan_includes
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:605 CppCompileActionBuilder.setCoptsFilter
    pub fn set_copts_filter(&mut self, copts_filter: CoptsFilter) -> &mut Self {
        self.copts_filter = copts_filter;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:615 CppCompileActionBuilder.setBuildInfoHeaderArtifacts
    pub fn set_build_info_header_artifacts(&mut self, artifacts: Vec<Artifact>) -> &mut Self {
        self.build_info_header_artifacts = artifacts;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:622 CppCompileActionBuilder.setCacheKeyInputs
    pub fn set_cache_key_inputs(&mut self, cache_key_inputs: NestedSet<Artifact>) -> &mut Self {
        self.cache_key_inputs = cache_key_inputs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:632 CppCompileActionBuilder.setAdditionalPrunableHeaders
    pub fn set_additional_prunable_headers(&mut self, headers: NestedSet<Artifact>) -> &mut Self {
        self.additional_prunable_headers = headers;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:639 CppCompileActionBuilder.setModmapFile
    pub fn set_modmap_file(&mut self, modmap_file: Option<Artifact>) -> &mut Self {
        self.modmap_file = modmap_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:645 CppCompileActionBuilder.setModmapInputFile
    pub fn set_modmap_input_file(&mut self, modmap_input_file: Option<Artifact>) -> &mut Self {
        self.modmap_input_file = modmap_input_file;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:651 CppCompileActionBuilder.setModuleFiles
    pub fn set_module_files(&mut self, module_files: Option<NestedSet<Artifact>>) -> &mut Self {
        self.module_files = module_files;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_bits_match_java_uuid() {
        // UUID.fromString("97493805-894f-493a-be66-9a698f45c31d")
        assert_eq!(format!("{GUID_MOST:016x}"), "97493805894f493a");
        assert_eq!(format!("{GUID_LEAST:016x}"), "be669a698f45c31d");
    }

    #[test]
    fn mnemonics() {
        let fc = FeatureConfiguration::empty();
        assert_eq!(
            action_name_to_mnemonic(CppActionNames::CPP_COMPILE, &fc, false),
            "CppCompile"
        );
        assert_eq!(
            action_name_to_mnemonic(CppActionNames::C_COMPILE, &fc, false),
            "CppCompile"
        );
        assert_eq!(
            action_name_to_mnemonic(CppActionNames::CPP_HEADER_PARSING, &fc, true),
            "CppCompileHeader"
        );
        assert_eq!(
            action_name_to_mnemonic(CppActionNames::OBJC_COMPILE, &fc, false),
            "ObjcCompile"
        );
        assert_eq!(
            action_name_to_mnemonic(CppActionNames::LINKSTAMP_COMPILE, &fc, false),
            "CppLinkstampCompile"
        );
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/cpp/CcCommonTest.java testClangClParameters
    // PORT: the Java test builds the action through BuildViewTestCase; this checks the option
    // parsing of getSystemIncludeDirs / getIncludeDirs over the same copts.
    #[test]
    fn test_clang_cl_parameters() {
        let copts = strings(&[
            "/imsvc",
            "SYSTEM_INCLUDE_1",
            "-imsvcSYSTEM_INCLUDE_2",
            "/ISTANDARD_INCLUDE",
            "/FI",
            "forced_include_1",
            "-FIforced_include_2",
        ]);
        let system_include1 = PathFragment::create("SYSTEM_INCLUDE_1");
        let system_include2 = PathFragment::create("SYSTEM_INCLUDE_2");
        let standard_include = PathFragment::create("STANDARD_INCLUDE");

        let system = system_include_dirs_from_options(&copts, &|| "action".to_owned());
        assert!(system.contains(&system_include1));
        assert!(system.contains(&system_include2));
        assert!(!system.contains(&standard_include));

        let mut include_dirs = Vec::new();
        include_dirs_from_copts(&copts, &mut include_dirs);
        assert!(!include_dirs.contains(&system_include1));
        assert!(!include_dirs.contains(&system_include2));
        assert!(include_dirs.contains(&standard_include));
    }

    #[test]
    fn system_include_dirs_separate_and_joined_forms() {
        let options = strings(&["-isystem", "a/b", "-isystemc/d", "-isystem  e ", "-I", "x"]);
        assert_eq!(
            system_include_dirs_from_options(&options, &|| "action".to_owned()),
            vec![
                PathFragment::create("a/b"),
                PathFragment::create("c/d"),
                PathFragment::create("e"),
            ]
        );
        // A dangling flag is skipped (with a warning).
        assert!(
            system_include_dirs_from_options(&strings(&["-isystem"]), &|| "action".to_owned())
                .is_empty()
        );
    }

    #[test]
    fn include_dirs_require_combined_form() {
        let mut dirs = Vec::new();
        include_dirs_from_copts(&strings(&["-I", "a", "-Ib", "/Ic", "-Imsvcd"]), &mut dirs);
        assert_eq!(
            dirs,
            vec![PathFragment::create("b"), PathFragment::create("c")]
        );
    }

    #[test]
    fn estimate_resource_consumption_local_values() {
        let default = default_resource_set();
        assert_eq!(default.get_memory_mb(), 250.0);
        assert_eq!(default.get_cpu_usage(), 1.0);
        let disabled =
            estimate_resource_consumption_local(false, Some("CppCompile"), cc::OS::LINUX, 10);
        assert_eq!(disabled.get_memory_mb(), 250.0);
        let linux =
            estimate_resource_consumption_local(true, Some("CppCompile"), cc::OS::LINUX, 10);
        assert_eq!(linux.get_memory_mb(), 87.0);
        assert_eq!(linux.get_cpu_usage(), 1.0);
        let objc_linux =
            estimate_resource_consumption_local(true, Some("ObjcCompile"), cc::OS::LINUX, 10);
        assert_eq!(objc_linux.get_memory_mb(), 250.0);
        let objc_darwin =
            estimate_resource_consumption_local(true, Some("ObjcCompile"), cc::OS::DARWIN, 10);
        assert_eq!(objc_darwin.get_memory_mb(), 82.0);
        let none = estimate_resource_consumption_local(true, None, cc::OS::LINUX, 10);
        assert_eq!(none.get_memory_mb(), 250.0);
    }

    #[test]
    fn msvc_matchers() {
        assert!(matches_case_insensitive_msvc("mSvC"));
        assert!(matches_case_insensitive_msvc("msvcfoo"));
        assert!(!matches_case_insensitive_msvc("msv"));
        assert!(matches_include_case_insensitive_msvc("/imsvc"));
        assert!(matches_include_case_insensitive_msvc("-IMSVCdir"));
        assert!(!matches_include_case_insensitive_msvc("-isystem"));
        // Internal strings: "é" is two chars, "🐷" four, a raw byte FF one.
        let i = |s: &[u8]| blaze_util::string_encoding::latin1_decode(s).into_owned();
        assert!(matches_case_insensitive_msvc(&i("msvc\u{e9}".as_bytes())));
        assert!(!matches_case_insensitive_msvc(&i("m\u{e9}".as_bytes())));
        assert!(!matches_case_insensitive_msvc(&i(b"msv\xff")));
        assert!(matches_include_case_insensitive_msvc(&i(
            "-imsvc\u{1F437}".as_bytes()
        )));
        assert!(!matches_include_case_insensitive_msvc(&i(
            "-i\u{1F437}".as_bytes()
        )));
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException: Multiple entries with same key")]
    fn put_or_throw_rejects_duplicate_keys() {
        let mut m = IndexMap::new();
        m.insert("k".to_owned(), "v".to_owned());
        put_or_throw(&mut m, "k", "w".to_owned());
    }

    // Java: src/test/java/com/google/devtools/build/lib/rules/cpp/CcLibraryConfiguredTargetTest.java
    #[test]
    #[ignore = "needs BuildViewTestCase (getCppCompileAction of a configured cc_library)"]
    fn test_include_paths_outside_execution_root() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (getCppCompileAction of a configured cc_library)"]
    fn test_absolute_include_paths_outside_execution_root() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (getCppCompileAction of a configured cc_library)"]
    fn test_system_include_paths_outside_execution_root() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (getCppCompileAction of a configured cc_library)"]
    fn test_absolute_system_include_paths_outside_execution_root() {}

    #[test]
    fn cpp20_module_actions() {
        assert!(is_cpp20_module_compilation_action(
            CppActionNames::CPP20_MODULE_COMPILE
        ));
        assert!(!is_cpp20_module_compilation_action(
            CppActionNames::CPP_COMPILE
        ));
    }
}
