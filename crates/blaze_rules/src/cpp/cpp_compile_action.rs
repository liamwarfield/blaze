//! `CppCompileAction` and `CppCompileActionBuilder` (analysis-time parts): the action's inputs,
//! outputs, environment, execution info, command line, mnemonic and action key.
//!
//! PORT: execution-time behaviour (`discoverInputs`, `execute`, `.d` file parsing, include
//! validation, `updateInputs`) belongs to the execution layer and is not ported here; the
//! Starlark-independent pieces exist in `blaze_cc_features` (`header_discovery`,
//! `dependency_set`, `cpp_compile_action::validate_inclusions`).

use std::any::Any;
use std::sync::{Arc, OnceLock};

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::fingerprint_util::add_paths;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::path_mappers;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragments::cpp_configuration::CppConfiguration;
use blaze_cc_features::actions::NOOP as CC_NOOP;
use blaze_cc_features::cc_common::CoptsFilter;
use blaze_cc_features::cc_toolchain_features::FeatureConfiguration;
use blaze_cc_features::cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
use blaze_cc_features::compile_build_variables::CompileBuildVariables;
use blaze_cc_features::compile_command_line::CompileCommandLine;
use blaze_cc_features::cpp_action_names::CppActionNames;
use blaze_cc_features::cpp_rule_classes;
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_shell::ShellEscaper;
use blaze_util::{Fingerprint, check_state};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use super::cc_compilation_context::CcCompilationContext;
use super::cpp_file_types;

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

fn cc_error(e: blaze_cc_features::actions::CommandLineExpansionException) -> MapFnError {
    MapFnError::CommandLineExpansion(CommandLineExpansionException::new(e.message))
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
    /// Used modules that are not transitively used through other topLevelModules.
    top_level_modules: Option<NestedSet<Artifact>>,
    /// Set only for C++ module compiles (see Java).
    discovered_modules: Option<NestedSet<Artifact>>,
    param_file_path: Option<PathFragment>,
    module_files: Option<NestedSet<Artifact>>,
    modmap_input_file: Option<Artifact>,
    /// `configuration.getActionEnvironment()`.
    env: Arc<ActionEnvironment>,
    /// `PathMappers.getOutputPathsMode(configuration)`.
    output_paths_mode: OutputPathsMode,
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
            top_level_modules: None,
            discovered_modules: None,
            param_file_path,
            module_files,
            modmap_input_file,
            env,
            output_paths_mode,
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
        self.top_level_modules = Some(top_level_modules.clone());
        if cpp_file_types::CPP_MODULE.matches_artifact(&primary_output)
            && !is_cpp20_module_compilation_action(&self.action_name)
        {
            let mut b = NestedSetBuilder::from_nested_set(
                self.cc_compilation_context
                    .get_transitive_modules(self.use_pic),
            );
            b.add_transitive(&top_level_modules);
            self.discovered_modules = Some(b.build());
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

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1305 CppCompileAction.getAllowedDerivedInputs
    pub fn get_allowed_derived_inputs(&self) -> &NestedSet<Artifact> {
        &self.allowed_derived_inputs
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
            match &self.top_level_modules {
                Some(modules) => Self::calculate_module_variable(modules),
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
fn build_command_line(
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

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:539 CppCompileAction.discoversInputs
    fn discovers_inputs(&self) -> bool {
        is_cpp20_module_compilation_action(&self.action_name)
            || self.should_scan_includes
            || self.get_dotd_file().is_some()
            || self.should_parse_show_includes()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:547 CppCompileAction.inputsDiscovered
    // PORT: input discovery runs at execution; at analysis inputs are never discovered.
    fn inputs_discovered(&self) -> bool {
        false
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
        }
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
        Ok(CppCompileAction::new(
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
        ))
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
