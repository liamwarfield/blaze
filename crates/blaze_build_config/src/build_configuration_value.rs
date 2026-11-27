//! Port of `analysis/config/BuildConfigurationValue.java` (the data and methods configured
//! targets and commands read; the Starlark API comes with the analysis wave) and
//! `analysis/config/FeatureSet.java`.

// PORT: skipped, no module posts it at the pin (CommandEnvironment.additionalConfigurationChangeEvent has no caller): AdditionalConfigurationChangeEvent.<init>, AdditionalConfigurationChangeEvent.getChangeDescription
// PORT: skipped, no subscriber at the pin (BuildView posts it; blaze_sky_top's BuildView.update does not): TopLevelConfigRequestedEvent.<init>

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::compilation_mode::CompilationMode;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_options::value::OptionScalar;
use blaze_options::{EnvVar, TriState};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::action_environment::ActionEnvironment;
use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassSet, FragmentFactory, FragmentRegistry,
    InvalidConfigurationException,
};
use crate::fragment_options_class::TEST_OPTIONS;
use crate::output_directories::{ArtifactRoot, ConfigDirectories, OutputDirectories};

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:96 BuildConfigurationValue.GlobalStateProvider
/// Global state necessary to build a BuildConfiguration (Java: the `ConfiguredRuleClassProvider`).
pub trait GlobalStateProvider: Send + Sync {
    /// Computes the default shell environment for actions from the command line options.
    fn get_action_environment(&self, options: &BuildOptions) -> ActionEnvironment;

    fn get_fragment_registry(&self) -> &FragmentRegistry;

    fn get_reserved_action_mnemonics(&self) -> &[String];

    fn get_runfiles_prefix(&self) -> &str;
}

// Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:35 FeatureSet
/// Represents a set of "on" features and a set of "off" features. The two sets are guaranteed
/// not to intersect.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FeatureSet {
    pub on: BTreeSet<String>,
    pub off: BTreeSet<String>,
}

impl FeatureSet {
    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:48 FeatureSet.parse
    pub fn parse<'a>(features: impl IntoIterator<Item = &'a String>) -> FeatureSet {
        let mut feature_to_state: HashMap<String, bool> = HashMap::new();
        for feature in features {
            if let Some(stripped) = feature.strip_prefix('-') {
                feature_to_state.insert(stripped.to_string(), false);
            } else if feature == "no_layering_check" {
                // TODO(bazel-team): Remove once we do not have BUILD files left that contain
                // 'no_layering_check'.
                feature_to_state.insert("layering_check".to_string(), false);
            } else {
                // -X always trumps X.
                feature_to_state.entry(feature.clone()).or_insert(true);
            }
        }
        Self::from_map(&feature_to_state)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:65 FeatureSet.fromMap
    fn from_map(feature_to_state: &HashMap<String, bool>) -> FeatureSet {
        // PORT: ImmutableSortedSet.copyOf (natural String order; features are ASCII).
        FeatureSet {
            on: feature_to_state
                .iter()
                .filter(|(_, v)| **v)
                .map(|(k, _)| k.clone())
                .collect(),
            off: feature_to_state
                .iter()
                .filter(|(_, v)| !**v)
                .map(|(k, _)| k.clone())
                .collect(),
        }
    }

    fn merge_set_into_map(
        features: &BTreeSet<String>,
        state: bool,
        feature_to_state: &mut HashMap<String, bool>,
    ) {
        for feature in features {
            feature_to_state.insert(feature.clone(), state);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:83 FeatureSet.merge
    pub fn merge(coarse: &FeatureSet, fine: &FeatureSet) -> FeatureSet {
        let mut m = HashMap::new();
        Self::merge_set_into_map(&coarse.on, true, &mut m);
        Self::merge_set_into_map(&coarse.off, false, &mut m);
        Self::merge_set_into_map(&fine.on, true, &mut m);
        Self::merge_set_into_map(&fine.off, false, &mut m);
        Self::from_map(&m)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:96 FeatureSet.mergeWithGlobalFeatures
    pub fn merge_with_global_features(base: &FeatureSet, global: &FeatureSet) -> FeatureSet {
        let mut m = HashMap::new();
        Self::merge_set_into_map(&global.on, true, &mut m);
        Self::merge_set_into_map(&base.on, true, &mut m);
        Self::merge_set_into_map(&base.off, false, &mut m);
        Self::merge_set_into_map(&global.off, false, &mut m);
        Self::from_map(&m)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:105 FeatureSet.toStringList
    pub fn to_string_list(&self) -> Vec<String> {
        self.on
            .iter()
            .cloned()
            .chain(self.off.iter().map(|s| format!("-{}", s)))
            .collect()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:861 BuildConfigurationValue.RunfileSymlinksMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum RunfileSymlinksMode {
    SKIP,
    CREATE,
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:867 BuildConfigurationValue.getRunfileSymlinksMode
pub fn get_runfile_symlinks_mode(options: &CoreOptions) -> RunfileSymlinksMode {
    // PORT: Linux only (`OS.getCurrent() != OS.WINDOWS` is true).
    if options.enable_runfiles == TriState::YES || options.enable_runfiles == TriState::AUTO {
        return RunfileSymlinksMode::CREATE;
    }
    RunfileSymlinksMode::SKIP
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:880 BuildConfigurationValue.runfilesEnabled
pub fn runfiles_enabled(options: &CoreOptions) -> bool {
    get_runfile_symlinks_mode(options) == RunfileSymlinksMode::CREATE
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:89 BuildConfigurationValue
/// Represents a collection of context information which may affect a build (for example: the
/// target platform for compilation, or whether or not debug tables are required). In fact, all
/// "environmental" information (e.g. from the tool's command-line, as opposed to the BUILD file)
/// that can affect the output of any build tool should be explicitly represented in the
/// `BuildConfigurationValue` instance.
pub struct BuildConfigurationValue {
    output_directories: OutputDirectories,
    fragments: Vec<(FragmentClass, Arc<dyn Fragment>)>,
    starlark_visible_fragments: IndexMap<&'static str, FragmentClass>,
    workspace_name: String,
    reserved_action_mnemonics: Vec<String>,
    /// `CommandLineLimits(options.minParamFileSize)`.
    command_line_limits_max_length: i32,
    /// The global "make variables" such as "$(TARGET_CPU)"; these get applied to all rules
    /// analyzed in this configuration.
    global_make_env: IndexMap<String, String>,
    action_env: ActionEnvironment,
    test_env: ActionEnvironment,
    build_options: Arc<BuildOptions>,
    /// The cpu value based on the platform the configuration is built for.
    platform_cpu: String,
    /// If non-empty, this is appended to output directories as
    /// ST-[transitionDirectoryNameFragment].
    mnemonic: String,
    command_line_build_variables: IndexMap<String, String>,
    sibling_repository_layout: bool,
    default_features: FeatureSet,
}

impl BuildConfigurationValue {
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:155 BuildConfigurationValue.reportInvalidOptions
    /// Validates the options for this BuildConfigurationValue. Issues warnings for the use of
    /// deprecated options, and warnings or errors for any option settings that conflict.
    pub fn report_invalid_options(&self, reporter: &dyn EventHandler) {
        // Validate that --cpu has an allowed value. Since there is no CoreConfiguration, handle
        // this directly instead of using reportInvalidOptions.
        let core_options = self.options();
        if !core_options.allowed_cpu_values.is_empty()
            && !core_options.allowed_cpu_values.contains(&core_options.cpu)
        {
            reporter.handle(&Event::error(format!(
                "Invalid --cpu value \"{}\": allowed values are {}.",
                core_options.cpu,
                core_options.allowed_cpu_values.join(", ")
            )));
        }

        for (_, fragment) in &self.fragments {
            fragment.report_invalid_options(reporter, &self.build_options);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:180 BuildConfigurationValue.setupTestEnvironment
    /// Compute the test environment, which, at configuration level, is a pair consisting of the
    /// statically set environment variables with their values and the set of environment
    /// variables to be inherited from the client environment.
    fn setup_test_environment(build_options: &BuildOptions) -> ActionEnvironment {
        if !build_options.contains(&TEST_OPTIONS) {
            // TestOptions have been trimmed.
            return ActionEnvironment::empty();
        }
        // Order doesn't matter here as ActionEnvironment sorts by key.
        let mut test_env: BTreeMap<String, Option<String>> = BTreeMap::new();
        for env_var in &build_options
            .get::<TestConfigurationTestOptions>()
            .unwrap()
            .test_environment
        {
            match env_var {
                EnvVar::Set { name, value } => {
                    test_env.insert(name.clone(), Some(value.clone()));
                }
                EnvVar::Inherit { name } => {
                    test_env.insert(name.clone(), None);
                }
                EnvVar::Unset { name } => {
                    test_env.remove(name);
                }
            }
        }
        ActionEnvironment::split(&test_env)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:198 BuildConfigurationValue.create
    /// Only BuildConfigurationFunction should instantiate this.
    pub fn create(
        build_options: Arc<BuildOptions>,
        baseline_options: Option<&BuildOptions>,
        sibling_repository_layout: bool,
        platform_cpu: String,
        // Arguments below this are server-global.
        directories: &ConfigDirectories,
        global_provider: &dyn GlobalStateProvider,
        fragment_factory: &FragmentFactory,
    ) -> Result<BuildConfigurationValue, InvalidConfigurationException> {
        let fragment_classes = if build_options.has_no_config() {
            FragmentClassSet::of([])
        } else {
            global_provider
                .get_fragment_registry()
                .get_all_fragments()
                .clone()
        };
        let fragments =
            Self::get_configuration_fragments(&build_options, &fragment_classes, fragment_factory)?;

        let mnemonic = crate::output_path_mnemonic_computer::compute_mnemonic(
            &build_options,
            baseline_options,
            &fragments,
        )?;

        let action_environment = global_provider.get_action_environment(&build_options);
        Ok(Self::new(
            build_options,
            mnemonic,
            sibling_repository_layout,
            platform_cpu,
            global_provider.get_runfiles_prefix().to_string(),
            directories,
            fragments,
            global_provider.get_reserved_action_mnemonics().to_vec(),
            action_environment,
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:235 BuildConfigurationValue.createForTesting
    pub fn create_for_testing(
        build_options: Arc<BuildOptions>,
        mnemonic: &str,
        sibling_repository_layout: bool,
        directories: &ConfigDirectories,
        global_provider: &dyn GlobalStateProvider,
        fragment_factory: &FragmentFactory,
    ) -> Result<BuildConfigurationValue, InvalidConfigurationException> {
        let fragment_classes = if build_options.has_no_config() {
            FragmentClassSet::of([])
        } else {
            global_provider
                .get_fragment_registry()
                .get_all_fragments()
                .clone()
        };
        let fragments =
            Self::get_configuration_fragments(&build_options, &fragment_classes, fragment_factory)?;
        let action_environment = global_provider.get_action_environment(&build_options);
        Ok(Self::new(
            build_options,
            mnemonic.to_string(),
            sibling_repository_layout,
            String::new(),
            global_provider.get_runfiles_prefix().to_string(),
            directories,
            fragments,
            global_provider.get_reserved_action_mnemonics().to_vec(),
            action_environment,
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:214 BuildConfigurationValue.getConfigurationFragments
    fn get_configuration_fragments(
        build_options: &BuildOptions,
        fragment_classes: &FragmentClassSet,
        fragment_factory: &FragmentFactory,
    ) -> Result<Vec<(FragmentClass, Arc<dyn Fragment>)>, InvalidConfigurationException> {
        let mut fragments = Vec::new();
        for fragment_class in fragment_classes.iter() {
            if let Some(fragment) =
                fragment_factory.create_fragment(build_options, fragment_class)?
            {
                fragments.push((fragment_class, fragment));
            }
        }
        // ImmutableSortedMap.orderedBy(LEXICAL_FRAGMENT_SORTER): FragmentClassSet is sorted.
        Ok(fragments)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:279 BuildConfigurationValue.<init>
    fn new(
        build_options: Arc<BuildOptions>,
        mnemonic: String,
        sibling_repository_layout: bool,
        platform_cpu: String,
        // Arguments below this are either server-global and constant or completely dependent
        // values.
        workspace_name: String,
        directories: &ConfigDirectories,
        fragments: Vec<(FragmentClass, Arc<dyn Fragment>)>,
        reserved_action_mnemonics: Vec<String>,
        action_environment: ActionEnvironment,
    ) -> BuildConfigurationValue {
        let mut starlark_visible_fragments = IndexMap::new();
        for (fragment_class, _) in &fragments {
            if let Some(name) = fragment_class.starlark_name {
                if starlark_visible_fragments
                    .insert(name, *fragment_class)
                    .is_some()
                {
                    panic!(
                        "IllegalArgumentException: Multiple entries with same key: {}",
                        name
                    );
                }
            }
        }
        let options = build_options.get::<CoreOptions>().expect("CoreOptions");
        let output_directories = OutputDirectories::new(
            directories,
            options,
            build_options.get::<PlatformOptions>(),
            &mnemonic,
            &workspace_name,
            sibling_repository_layout,
        );

        // We can't use an ImmutableMap.Builder here; we need the ability to add entries with keys
        // that are already in the map so that the same define can be specified on the command
        // line twice, and ImmutableMap.Builder does not support that.
        let command_line_build_variables: IndexMap<String, String> = options
            .get_normalized_command_line_build_variables()
            .into_iter()
            .collect();

        let test_env = Self::setup_test_environment(&build_options);

        // These should be documented in the build encyclopedia.
        // TODO(configurability-team): Deprecate TARGET_CPU in favor of platforms.
        let main = blaze_cmdline::repository_name::MAIN.clone();
        let mut global_make_env = IndexMap::new();
        global_make_env.insert(
            "TARGET_CPU".to_string(),
            if options.incompatible_target_cpu_from_platform {
                platform_cpu.clone()
            } else {
                options.cpu.clone()
            },
        );
        global_make_env.insert(
            "COMPILATION_MODE".to_string(),
            options.compilation_mode.java_to_string(),
        );
        global_make_env.insert(
            "BINDIR".to_string(),
            output_directories
                .get_bin_directory(&main)
                .get_exec_path_string(),
        );
        global_make_env.insert(
            "GENDIR".to_string(),
            output_directories
                .get_genfiles_directory(&main)
                .get_exec_path_string(),
        );

        let command_line_limits_max_length = options.min_param_file_size;
        let default_features = FeatureSet::parse(&options.default_features);

        BuildConfigurationValue {
            output_directories,
            fragments,
            starlark_visible_fragments,
            workspace_name,
            reserved_action_mnemonics,
            command_line_limits_max_length,
            global_make_env,
            action_env: action_environment,
            test_env,
            build_options,
            platform_cpu,
            mnemonic,
            command_line_build_variables,
            sibling_repository_layout,
            default_features,
        }
    }

    /// `options` (the `CoreOptions` of the configuration).
    fn options(&self) -> &CoreOptions {
        self.build_options
            .get::<CoreOptions>()
            .expect("CoreOptions")
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:379 BuildConfigurationValue.getKey
    /// Returns the `BuildConfigurationKey` for this configuration.
    pub fn get_key(&self) -> crate::build_configuration_key::BuildConfigurationKey {
        crate::build_configuration_key::BuildConfigurationKey::create(self.build_options.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:389 BuildConfigurationValue.getOutputDirectory
    pub fn get_output_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        self.output_directories
            .get_output_directory(repository_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:331 BuildConfigurationValue.getBinDirectory
    pub fn get_bin_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        self.output_directories.get_bin_directory(repository_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:423 BuildConfigurationValue.getBinFragment
    pub fn get_bin_fragment(&self, repository_name: &RepositoryName) -> PathFragment {
        self.get_bin_directory(repository_name)
            .get_exec_path()
            .clone()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:333 BuildConfigurationValue.getGenfilesDirectory
    pub fn get_genfiles_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        self.output_directories
            .get_genfiles_directory(repository_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:446 BuildConfigurationValue.hasSeparateGenfilesDirectory
    pub fn has_separate_genfiles_directory(&self) -> bool {
        !self.output_directories.merge_genfiles_directory()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:462 BuildConfigurationValue.getTestLogsDirectory
    pub fn get_test_logs_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        self.output_directories
            .get_test_logs_directory(repository_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:472 BuildConfigurationValue.getGenfilesFragment
    pub fn get_genfiles_fragment(&self, repository_name: &RepositoryName) -> PathFragment {
        self.output_directories
            .get_genfiles_fragment(repository_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:483 BuildConfigurationValue.getHostPathSeparator
    pub fn get_host_path_separator(&self) -> &'static str {
        self.output_directories.get_host_path_separator()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:487 BuildConfigurationValue.isStrictFilesets
    pub fn is_strict_filesets(&self) -> bool {
        self.options().strict_filesets
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:500 BuildConfigurationValue.getMnemonic
    /// Returns the configuration-dependent string for this configuration.
    pub fn get_mnemonic(&self) -> &str {
        self.output_directories.get_mnemonic()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:515 BuildConfigurationValue.getOutputDirectoryName
    pub fn get_output_directory_name(&self) -> &str {
        self.output_directories.get_output_dir_name()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:495 BuildConfigurationValue.getWorkspaceName
    pub fn get_workspace_name(&self) -> &str {
        &self.workspace_name
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:98 BuildConfigurationValue.getActionEnvironment
    /// Returns the action environment: the fixed part plus the names inherited from the client.
    pub fn get_action_environment(&self) -> &ActionEnvironment {
        &self.action_env
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:611 BuildConfigurationValue.getMakeEnvironment
    /// Returns a (key, value) mapping to insert into the subcommand environment for testing.
    pub fn get_make_environment(&self) -> IndexMap<String, String> {
        // ImmutableMap.Builder.buildKeepingLast: a repeated key keeps its first position and
        // takes the last value.
        let mut make_environment = self.global_make_env.clone();
        for (k, v) in &self.command_line_build_variables {
            make_environment.insert(k.clone(), v.clone());
        }
        make_environment
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:622 BuildConfigurationValue.getCommandLineBuildVariables
    pub fn get_command_line_build_variables(&self) -> &IndexMap<String, String> {
        &self.command_line_build_variables
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:627 BuildConfigurationValue.getGlobalMakeEnvironment
    pub fn get_global_make_environment(&self) -> &IndexMap<String, String> {
        &self.global_make_env
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:640 BuildConfigurationValue.getFragment
    pub fn get_fragment<T: Fragment>(&self) -> Option<&T> {
        self.fragments
            .iter()
            .find_map(|(_, f)| f.as_any().downcast_ref::<T>())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:645 BuildConfigurationValue.getFragments
    pub fn get_fragments(&self) -> &[(FragmentClass, Arc<dyn Fragment>)] {
        &self.fragments
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:650 BuildConfigurationValue.hasFragment
    pub fn has_fragment(&self, class: FragmentClass) -> bool {
        self.fragments.iter().any(|(c, _)| *c == class)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:668 BuildConfigurationValue.platformCpu
    pub fn platform_cpu(&self) -> &str {
        &self.platform_cpu
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:673 BuildConfigurationValue.stampBinaries
    pub fn stamp_binaries(&self) -> bool {
        self.options().stamp_binaries
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:710 BuildConfigurationValue.getTestEnv
    pub fn get_test_env(&self) -> &BTreeMap<String, String> {
        self.test_env.get_fixed_env()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:721 BuildConfigurationValue.getTestActionEnvironment
    pub fn get_test_action_environment(&self) -> &ActionEnvironment {
        &self.test_env
    }

    /// `getCommandLineLimits().maxLength`.
    pub fn get_command_line_limits_max_length(&self) -> i32 {
        self.command_line_limits_max_length
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:751 BuildConfigurationValue.isExecConfiguration
    pub fn is_exec_configuration(&self) -> bool {
        self.options().is_exec
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:756 BuildConfigurationValue.isToolConfiguration
    pub fn is_tool_configuration(&self) -> bool {
        self.is_exec_configuration()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:809 BuildConfigurationValue.getCompilationMode
    pub fn get_compilation_mode(&self) -> CompilationMode {
        self.options().compilation_mode
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:814 BuildConfigurationValue.checksum
    pub fn checksum(&self) -> &str {
        self.build_options.checksum()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:823 BuildConfigurationValue.shortId
    pub fn short_id(&self) -> String {
        self.build_options.short_id()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:828 BuildConfigurationValue.cloneOptions
    pub fn clone_options(&self) -> BuildOptions {
        self.build_options.as_ref().clone()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:159 BuildConfigurationValue.getOptions
    pub fn get_options(&self) -> &Arc<BuildOptions> {
        &self.build_options
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:846 BuildConfigurationValue.getCpu
    pub fn get_cpu(&self) -> &str {
        &self.options().cpu
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:867 BuildConfigurationValue.getRunfileSymlinksMode
    pub fn get_runfile_symlinks_mode(&self) -> RunfileSymlinksMode {
        get_runfile_symlinks_mode(self.options())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:880 BuildConfigurationValue.runfilesEnabled
    pub fn runfiles_enabled(&self) -> bool {
        runfiles_enabled(self.options())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:961 BuildConfigurationValue.getDefaultFeatures
    pub fn get_default_features(&self) -> &FeatureSet {
        &self.default_features
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:980 BuildConfigurationValue.getStarlarkFragmentByName
    pub fn get_starlark_fragment_by_name(&self, name: &str) -> Option<FragmentClass> {
        self.starlark_visible_fragments.get(name).copied()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:984 BuildConfigurationValue.getStarlarkFragmentNames
    pub fn get_starlark_fragment_names(&self) -> impl Iterator<Item = &&'static str> {
        self.starlark_visible_fragments.keys()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:102 BuildConfigurationValue.getReservedActionMnemonics
    pub fn get_reserved_action_mnemonics(&self) -> &[String] {
        &self.reserved_action_mnemonics
    }

    pub fn sibling_repository_layout(&self) -> bool {
        self.sibling_repository_layout
    }
}

// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:341 BuildConfigurationValue.equals
impl PartialEq for BuildConfigurationValue {
    fn eq(&self, other: &Self) -> bool {
        // Only considering arguments that are non-dependent and non-server-global.
        *self.build_options == *other.build_options
            && self.workspace_name == other.workspace_name
            && self.sibling_repository_layout == other.sibling_repository_layout
            && self.mnemonic == other.mnemonic
    }
}
impl Eq for BuildConfigurationValue {}

impl fmt::Debug for BuildConfigurationValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildConfigurationValue({}, {})",
            self.checksum(),
            self.mnemonic
        )
    }
}
