//! The configuration SkyFunctions: BUILD_CONFIGURATION (`BuildConfigurationFunction`),
//! BUILD_CONFIGURATION_KEY (`BuildConfigurationKeyFunction` with `BuildConfigurationKeyProducer`),
//! BASELINE_OPTIONS (`BaselineOptionsFunction`), PLATFORM_MAPPING (`PlatformMappingFunction`,
//! `PlatformMappingValue`), and the top-level configuration creation of `SkyframeExecutor`
//! (`setBaselineConfiguration`, `createConfiguration`, `getConfiguration`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::fragment::core_options::{CoreOptions, CoreOptionsOutputDirectoryNamingScheme};
use blaze_optdefs::fragment::platform_mapping_key::PlatformMappingKey;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::{Foreign, OptionsParsingException};
use blaze_skyframe::{
    ComputeError, Environment, EvaluationContext, EvaluatorBuilder, Key, SkyFunctionException,
    SkyFunctionName, SkyKey, Transience,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::Path;

use crate::build_configuration_key::{BuildConfigurationKey, BuildConfigurationKeyValueKey};
use crate::build_configuration_value::{BuildConfigurationValue, GlobalStateProvider};
use crate::build_options::BuildOptions;
use crate::fragment::{FragmentFactory, InvalidConfigurationException};
use crate::output_directories::ConfigDirectories;
use crate::platform::{PlatformValueKey, SharedPlatformValueProvider};

impl Key for BuildConfigurationKey {
    type Value = BuildConfigurationValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_CONFIGURATION");
}

impl Key for BuildConfigurationKeyValueKey {
    /// `BuildConfigurationKeyValue.buildConfigurationKey()`.
    type Value = BuildConfigurationKey;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_CONFIGURATION_KEY");
}

// Java: com/google/devtools/build/lib/analysis/config/transitions/BaselineOptionsValue.java:68 BaselineOptionsValue.Key
/// `BaselineOptionsValue.key(afterExecTransition, newPlatform)`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BaselineOptionsKey {
    pub after_exec_transition: bool,
    pub new_platform: Option<Label>,
}

impl fmt::Debug for BaselineOptionsKey {
    // Java: com/google/devtools/build/lib/analysis/config/transitions/BaselineOptionsValue.java:76 Key.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BaselineOptionsValue.Key{{afterExecTransition={}, newPlatform={}}}",
            self.after_exec_transition,
            Label::print(self.new_platform.as_ref())
        )
    }
}

impl Key for BaselineOptionsKey {
    /// `BaselineOptionsValue.toOptions()`.
    type Value = Arc<BuildOptions>;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BASELINE_OPTIONS");
}

/// The SkyKey of `PlatformMappingValue` (Java: `PlatformMappingKey` is itself the key).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PlatformMappingSkyKey(pub PlatformMappingKey);

impl fmt::Debug for PlatformMappingSkyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            blaze_options::value::OptionScalar::java_to_string(&self.0)
        )
    }
}

impl Key for PlatformMappingSkyKey {
    type Value = PlatformMappingValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("PLATFORM_MAPPING");
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:50 PlatformMappingValue
/// Stores contents of a platforms/flags mapping file for transforming one `BuildOptions` into
/// another.
///
/// PORT: only the empty mapping (no `platform_mappings` file) is ported; parsed mappings need
/// PARSED_FLAGS (`ParsedFlagsValue.mergeWith`, `BuildOptions.matches`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformMappingValue {
    /// The number of `platforms:` and `flags:` entries (only empty mappings are ported).
    is_empty: bool,
}

impl PlatformMappingValue {
    pub fn empty() -> Self {
        PlatformMappingValue { is_empty: true }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:117 PlatformMappingValue.computeMapping
    /// Maps one `BuildOptions` to another by way of mappings provided in a file.
    pub fn map(
        &self,
        original_options: &BuildOptions,
    ) -> Result<BuildConfigurationKey, OptionsParsingException> {
        if original_options.has_no_config() {
            return Ok(BuildConfigurationKey::create(Arc::new(
                original_options.clone(),
            )));
        }
        let platform_options = original_options.get::<PlatformOptions>();
        blaze_util::check_argument!(
            platform_options.is_some(),
            "When using platform mappings, all configurations must contain platform options"
        );
        let platform_options = platform_options.unwrap();

        if !platform_options.platforms.is_empty() {
            // platformsToFlags.containsKey(targetPlatform)
            if !self.is_empty {
                blaze_util::unported!("PlatformMappingValue.computeMapping", "platforms: mappings");
            }
            return Ok(BuildConfigurationKey::create(Arc::new(
                original_options.clone(),
            )));
        }

        // for (flagsToPlatform : flagsToPlatforms) ... originalOptions.matches(...)
        if !self.is_empty {
            blaze_util::unported!("PlatformMappingValue.computeMapping", "flags: mappings");
        }

        let target_platform = platform_options.compute_target_platform().clone();
        let mut modified_options = original_options.clone();
        modified_options
            .get_mut::<PlatformOptions>()
            .unwrap()
            .platforms = vec![Foreign(target_platform)];
        Ok(BuildConfigurationKey::create(Arc::new(modified_options)))
    }
}

/// Errors of the configuration functions (Java exception types wrapped in
/// `SkyFunctionException`s).
#[derive(Clone, Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("{0}")]
    InvalidConfiguration(#[from] InvalidConfigurationException),
    #[error("{}", .0.get_message())]
    OptionsParsing(OptionsParsingException),
    /// `PlatformMappingException` / `MissingInputFileException`.
    #[error("{0}")]
    PlatformMapping(String),
    #[error("{0}")]
    StarlarkExecTransitionLoading(
        #[from] crate::starlark_transition::StarlarkExecTransitionLoadingException,
    ),
}

/// The evaluator client the configuration functions read (Java: the `SkyframeExecutor` state and
/// precomputed values they use).
pub struct ConfigurationEnvironment {
    pub directories: ConfigDirectories,
    pub rule_class_provider: Arc<dyn GlobalStateProvider>,
    /// `PrecomputedValue.STARLARK_SEMANTICS`.
    pub starlark_semantics: Arc<StarlarkSemantics>,
    /// `PrecomputedValue.PATH_PACKAGE_LOCATOR` path entries (for the platform mappings file).
    pub package_path_roots: Vec<Path>,
    /// `PrecomputedValue.BASELINE_CONFIGURATION`.
    pub baseline_configuration: Arc<BuildOptions>,
    /// `PrecomputedValue.BASELINE_EXEC_CONFIGURATION`.
    pub baseline_exec_configuration: Arc<BuildOptions>,
    pub platforms: SharedPlatformValueProvider,
}

fn client<'a>(env: &'a Environment<'_>) -> &'a ConfigurationEnvironment {
    env.client::<ConfigurationEnvironment>()
        .expect("ConfigurationEnvironment client")
}

fn persistent(e: impl Into<ConfigurationError>) -> ComputeError {
    SkyFunctionException::new(e.into(), Transience::Persistent).into()
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:70 PlatformMappingFunction.compute
fn compute_platform_mapping(
    key: &PlatformMappingSkyKey,
    env: &mut Environment<'_>,
) -> Result<Option<PlatformMappingValue>, ComputeError> {
    let workspace_relative_mapping_path = key.0.get_workspace_relative_mapping_path().clone();
    // PORT: the main repository mapping is only needed to parse a mappings file.
    let path_entries = client(env).package_path_roots.clone();
    for root in &path_entries {
        // PORT: Java requests FileValue.key(rootedMappingPath) (FILE in blaze_sky_loading); this
        // reads the file system directly until the configuration functions run in the shared
        // evaluator.
        let path = root.get_relative(&workspace_relative_mapping_path);
        let std_path = std::path::Path::new(path.get_path_string());
        let Ok(metadata) = std::fs::metadata(std_path) else {
            continue;
        };
        if metadata.is_dir() {
            return Err(persistent(ConfigurationError::PlatformMapping(format!(
                "--platform_mappings was set to '{}' relative to the top-level workspace '{}' but that path refers to a directory, not a file",
                workspace_relative_mapping_path, root
            ))));
        }
        let text = std::fs::read_to_string(std_path)
            .map_err(|e| persistent(ConfigurationError::PlatformMapping(e.to_string())))?;
        let has_entries = text
            .lines()
            .map(str::trim)
            .any(|line| !line.is_empty() && !line.starts_with('#'));
        if !has_entries {
            return Ok(Some(PlatformMappingValue::empty()));
        }
        blaze_util::unported!("PlatformMappingFunction.parse", "{}", path);
    }

    if !key.0.was_explicitly_set_by_user() {
        // If no flag was passed and the default mapping file does not exist treat this as if
        // the mapping file was empty rather than an error.
        return Ok(Some(PlatformMappingValue::empty()));
    }
    Err(persistent(ConfigurationError::PlatformMapping(format!(
        "--platform_mappings was set to '{}' but no such file exists relative to the package path roots, '[{}]'",
        workspace_relative_mapping_path,
        path_entries
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))))
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyFunction.java:42 BuildConfigurationKeyFunction.compute
/// Delegates to `BuildConfigurationKeyProducer` (inlined: the producer's steps run in order,
/// returning `None` where a Skyframe restart is needed).
fn compute_build_configuration_key(
    key: &BuildConfigurationKeyValueKey,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    let options = key.build_options();
    // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:122 BuildConfigurationKeyProducer.step
    // Short-circuit if there are no platform options.
    let Some(platform_options) = options.get::<PlatformOptions>() else {
        return find_build_options_scopes(options.as_ref().clone());
    };

    let target_platforms = &platform_options.platforms;
    if target_platforms.len() == 1 {
        // TODO: https://github.com/bazelbuild/bazel/issues/19807 - We define this flag to only
        //  use the first value and ignore any subsequent ones.
        let platform_key = PlatformValueKey {
            platform_label: target_platforms[0].0.clone(),
            flag_alias_mappings: options
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .get_command_line_flag_aliases(),
        };
        let Some(target_platform_value) = env.get_value(platform_key)? else {
            return Ok(None);
        };
        // Java: BuildConfigurationKeyProducer.checkTargetPlatformFlags
        if target_platform_value.has_parsed_flags {
            blaze_util::unported!(
                "ParsedFlagsValue.mergeWith",
                "platform-based flags of {}",
                target_platform_value.label
            );
        }
        merge_from_platform_mapping(options, env)
    } else {
        blaze_util::check_state!(target_platforms.is_empty());
        merge_from_platform_mapping(options, env)
    }
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:209 BuildConfigurationKeyProducer.mergeFromPlatformMapping
fn merge_from_platform_mapping(
    options: &Arc<BuildOptions>,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    let mapping_key = PlatformMappingSkyKey(
        options
            .get::<PlatformOptions>()
            .unwrap()
            .platform_mapping_key
            .clone(),
    );
    let platform_mapping_value =
        match env.get_value_or_throw::<_, ConfigurationError>(mapping_key)? {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(None),
            Err(e) => return Err(persistent(e)),
        };
    // Java: BuildConfigurationKeyProducer.applyPlatformMapping
    match platform_mapping_value.map(options) {
        Ok(key) => find_build_options_scopes(key.get_options().as_ref().clone()),
        Err(e) => Err(persistent(ConfigurationError::OptionsParsing(e))),
    }
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:170 BuildConfigurationKeyProducer.findBuildOptionsScopes
fn find_build_options_scopes(
    post_platform_processed_options: BuildOptions,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    // including platform-based flags in skykey for scopes lookUp
    if !post_platform_processed_options
        .get_starlark_options()
        .is_empty()
    {
        // PORT: BUILD_OPTIONS_SCOPE (flag scoping) is not ported; no Starlark flags are set in
        // the configurations the corpus builds create.
        blaze_util::unported!(
            "BuildConfigurationKeyProducer.findBuildOptionsScopes",
            "Starlark options {:?}",
            post_platform_processed_options
                .get_starlark_options()
                .keys()
                .collect::<Vec<_>>()
        );
    }
    // possiblyApplyScopes -> finishConfigurationKeyProcessing
    Ok(Some(BuildConfigurationKey::create(Arc::new(
        post_platform_processed_options,
    ))))
}

// Java: com/google/devtools/build/lib/skyframe/config/BaselineOptionsFunction.java:43 BaselineOptionsFunction.compute
fn compute_baseline_options(
    key: &BaselineOptionsKey,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<BuildOptions>>, ComputeError> {
    let raw_baseline_options = if key.after_exec_transition {
        // Use the precomputed baseline exec
        client(env).baseline_exec_configuration.clone()
    } else {
        // Use the standard baseline
        client(env).baseline_configuration.clone()
    };

    // Some test infrastructure only creates mock or partial top-level BuildOptions such that
    // PlatformOptions or even CoreOptions might not be included.
    if raw_baseline_options.has_no_config() {
        return Ok(Some(raw_baseline_options));
    }

    // First, make sure platform_mappings applied to the top-level baseline option.
    let Some(mapped_baseline_options) = map_build_options(env, raw_baseline_options)? else {
        return Ok(None);
    };
    let mut adjusted_baseline_options = mapped_baseline_options;

    if let Some(new_platform) = &key.new_platform {
        // Clone for safety as-is the standard for all transitions.
        let mut cloned = adjusted_baseline_options.as_ref().clone();
        cloned.get_mut::<PlatformOptions>().unwrap().platforms =
            vec![Foreign(new_platform.clone())];
        adjusted_baseline_options = Arc::new(cloned);
    }

    // Re-apply platform_mappings if we updated the platform.
    let mut remapped_adjusted_baseline_options = adjusted_baseline_options;
    if key.new_platform.is_some() {
        match map_build_options(env, remapped_adjusted_baseline_options)? {
            Some(o) => remapped_adjusted_baseline_options = o,
            None => return Ok(None),
        }
    }
    Ok(Some(remapped_adjusted_baseline_options))
}

// Java: com/google/devtools/build/lib/skyframe/config/BaselineOptionsFunction.java:66 BaselineOptionsFunction.mapBuildOptions
fn map_build_options(
    env: &mut Environment<'_>,
    raw_baseline_options: Arc<BuildOptions>,
) -> Result<Option<Arc<BuildOptions>>, ComputeError> {
    let bckvk = BuildConfigurationKeyValueKey::create(raw_baseline_options);
    match env.get_value_or_throw::<_, ConfigurationError>(bckvk)? {
        Ok(Some(v)) => Ok(Some(v.get_options().clone())),
        Ok(None) => Ok(None),
        Err(e) => Err(persistent(e)),
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:57 BuildConfigurationFunction.compute
fn compute_build_configuration(
    key: &BuildConfigurationKey,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationValue>, ComputeError> {
    let starlark_semantics = client(env).starlark_semantics.clone();
    let target_options = key.get_options().clone();
    let Some(baseline_options) = get_baseline_options(env, &target_options)? else {
        return Ok(None);
    };

    let Some(platform_cpu) = get_platform_cpu(env, &target_options)? else {
        return Ok(None);
    };

    let cx = client(env);
    match BuildConfigurationValue::create(
        target_options,
        baseline_options.as_deref(),
        starlark_semantics.experimental_sibling_repository_layout,
        platform_cpu,
        // Arguments below this are server-global.
        &cx.directories,
        cx.rule_class_provider.as_ref(),
        &FragmentFactory,
    ) {
        // PORT: `env.getListener().post(ConfigurationValueEvent.create(value))` is skipped: the
        // event feeds BEP (wave W8).
        Ok(v) => Ok(Some(v)),
        Err(e) => Err(persistent(e)),
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:71 BuildConfigurationFunction.getPlatformCpu
fn get_platform_cpu(
    env: &mut Environment<'_>,
    target_options: &BuildOptions,
) -> Result<Option<String>, ComputeError> {
    let Some(platform_options) = target_options.get::<PlatformOptions>() else {
        return Ok(Some(String::new()));
    };
    let platform_label = platform_options.compute_target_platform().clone();

    let core_options = target_options.get::<CoreOptions>().expect("CoreOptions");
    if let Some(override_platform_cpu_name) =
        core_options.get_platform_cpu_name_override(&platform_label)
    {
        return Ok(Some(override_platform_cpu_name));
    }

    let Some(platform_value) = env.get_value(PlatformValueKey {
        platform_label,
        flag_alias_mappings: core_options.get_command_line_flag_aliases(),
    })?
    else {
        return Ok(None);
    };

    Ok(Some(
        match platform_value
            .constraints
            .get(&crate::platform::cpu_constraint_setting())
        {
            None => String::new(),
            Some(cpu_constraint) => cpu_constraint.get_name().to_string(),
        },
    ))
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:66 BuildConfigurationFunction.getBaselineOptions
/// Determine the baseline options to use for tracking changes. Returns `None` if a Skyframe
/// restart is needed, or `Some` with either the baseline options to use, or none if there is no
/// valid baseline.
fn get_baseline_options(
    env: &mut Environment<'_>,
    target_options: &BuildOptions,
) -> Result<Option<Option<Arc<BuildOptions>>>, ComputeError> {
    if target_options.has_no_config() {
        return Ok(Some(None));
    }
    let core_options = target_options.get::<CoreOptions>().expect("CoreOptions");
    if !core_options.use_baseline_for_output_directory_naming_scheme() {
        return Ok(Some(None));
    }
    let platform_options = target_options.get::<PlatformOptions>();

    // Determine whether this is part of the exec transition, or if we need to calculate a target
    // platform.
    let use_dynamic_baseline = core_options.output_directory_naming_scheme
        == CoreOptionsOutputDirectoryNamingScheme::DIFF_AGAINST_DYNAMIC_BASELINE;
    let apply_exec_transition_to_baseline = use_dynamic_baseline && core_options.is_exec;
    // In practice, platforms should always be 'well-formed' and contain at most one Label.
    let mut new_platform = None;
    if use_dynamic_baseline
        && let Some(platform_options) = platform_options
        && core_options.use_platform_in_output_dir(platform_options.compute_target_platform())
    {
        new_platform = Some(platform_options.compute_target_platform().clone());
    }

    match env.get_value_or_throw::<_, ConfigurationError>(BaselineOptionsKey {
        after_exec_transition: apply_exec_transition_to_baseline,
        new_platform,
    })? {
        Ok(Some(v)) => Ok(Some(Some(v.as_ref().clone()))),
        Ok(None) => Ok(None),
        Err(ConfigurationError::StarlarkExecTransitionLoading(e)) => {
            Err(persistent(InvalidConfigurationException::new(e.0.clone())))
        }
        Err(e) => Err(persistent(e)),
    }
}

/// Registers the configuration functions on an evaluator builder. The client must be a
/// [`ConfigurationEnvironment`].
pub fn register_configuration_functions(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .typed_function::<BuildConfigurationKey, _>(compute_build_configuration)
        .typed_function::<BuildConfigurationKeyValueKey, _>(compute_build_configuration_key)
        .typed_function::<BaselineOptionsKey, _>(compute_baseline_options)
        .typed_function::<PlatformMappingSkyKey, _>(compute_platform_mapping)
        .typed_function::<PlatformValueKey, _>(|key, env| {
            let platforms = client(env).platforms.clone();
            platforms.compute(key, env)
        })
}

// -------------------------------------------------------------------------------------------
// SkyframeExecutor.

/// What a configuration request needs from the command (Java: the `SkyframeExecutor` state).
pub struct ConfigurationRequest {
    pub directories: ConfigDirectories,
    pub rule_class_provider: Arc<dyn GlobalStateProvider>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub package_path_roots: Vec<Path>,
    pub platforms: SharedPlatformValueProvider,
}

/// The configuration-related state of `SkyframeExecutor` for one command.
///
/// PORT: Blaze has no shared `SkyframeExecutor` yet; configurations are computed in an evaluator
/// of their own, created once the baseline configuration is known (Java injects the baseline as
/// precomputed values into the shared graph). Move into the SkyframeExecutor port.
pub struct ConfigurationSkyframe {
    evaluator: blaze_skyframe::Evaluator,
    client: Arc<ConfigurationEnvironment>,
    event_handler: Arc<dyn ExtendedEventHandler>,
}

impl ConfigurationSkyframe {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1537 SkyframeExecutor.setBaselineConfiguration
    /// Sets the baseline configurations (and computes the exec baseline with the Starlark exec
    /// transition).
    pub fn set_baseline_configuration(
        request: ConfigurationRequest,
        build_options: Arc<BuildOptions>,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<ConfigurationSkyframe, InvalidConfigurationException> {
        let baseline_exec = Self::adjust_for_exec(
            &build_options,
            &request.starlark_semantics,
            event_handler.as_ref(),
        )?;
        let client = ConfigurationEnvironment {
            directories: request.directories,
            rule_class_provider: request.rule_class_provider,
            starlark_semantics: request.starlark_semantics,
            package_path_roots: request.package_path_roots,
            baseline_configuration: build_options,
            baseline_exec_configuration: Arc::new(baseline_exec),
            platforms: request.platforms,
        };
        let client = Arc::new(client);
        let evaluator = register_configuration_functions(EvaluatorBuilder::new())
            .client(client.clone())
            .build();
        Ok(ConfigurationSkyframe {
            evaluator,
            client,
            event_handler,
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1541 SkyframeExecutor.adjustForExec
    fn adjust_for_exec(
        build_options: &BuildOptions,
        semantics: &Arc<StarlarkSemantics>,
        event_handler: &dyn EventHandler,
    ) -> Result<BuildOptions, InvalidConfigurationException> {
        let exec_transition = crate::starlark_transition::load_starlark_exec_transition(
            Some(build_options),
            semantics,
        )
        .map_err(|e| InvalidConfigurationException::new(e.0))?;
        // Get the current target platform and use it as the exec platform.
        // This value isn't actually important as long as it exists and is stable.
        let host_platform = build_options
            .get::<PlatformOptions>()
            .map(|p| p.host_platform.0.clone());
        match exec_transition {
            Some(t) => Ok(crate::function_transition_util::adjust_for_exec(
                build_options,
                &t,
                host_platform.as_ref(),
                semantics,
                event_handler,
            )),
            None => blaze_util::unported!(
                "ExecutionTransitionFactory.create",
                "no Starlark exec transition (native exec transition is not in Bazel 9)"
            ),
        }
    }

    pub fn baseline_exec_configuration(&self) -> Arc<BuildOptions> {
        self.client().baseline_exec_configuration.clone()
    }

    fn client(&self) -> &ConfigurationEnvironment {
        &self.client
    }

    fn evaluate<K: Key + Clone>(
        &self,
        key: K,
        keep_going: bool,
    ) -> Result<Arc<K::Value>, InvalidConfigurationException> {
        let ctx = EvaluationContext {
            keep_going,
            event_handler: self.event_handler.clone(),
            ..EvaluationContext::default()
        };
        let sky_key = SkyKey::new(key.clone());
        let result = self
            .evaluator
            .evaluate(std::slice::from_ref(&sky_key), &ctx)
            .map_err(|_| InvalidConfigurationException::new("interrupted"))?;
        if let Some(v) = result.get(&key) {
            return Ok(v);
        }
        let (_, error) = &result.error_map()[0];
        if let Some(e) = error.exception_as::<ConfigurationError>() {
            return Err(match e {
                ConfigurationError::InvalidConfiguration(e) => e.clone(),
                other => InvalidConfigurationException::with_code(
                    other.to_string(),
                    "PLATFORM_MAPPING_EVALUATION_FAILURE",
                ),
            });
        }
        if !error.get_cycle_info().is_empty() {
            return Err(InvalidConfigurationException::with_code(
                "cannot load build configuration because of this cycle",
                "CYCLE",
            ));
        }
        panic!(
            "IllegalStateException: Unknown error during configuration creation evaluation: {:?}",
            error.get_exception()
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2113 SkyframeExecutor.createBuildConfigurationKey
    pub fn create_build_configuration_key(
        &self,
        build_options: Arc<BuildOptions>,
    ) -> Result<BuildConfigurationKey, InvalidConfigurationException> {
        Ok(self
            .evaluate(BuildConfigurationKeyValueKey::create(build_options), false)?
            .as_ref()
            .clone())
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1903 SkyframeExecutor.getConfiguration
    /// Returns the configuration corresponding to the given set of build options.
    pub fn get_configuration(
        &self,
        build_options: Arc<BuildOptions>,
        keep_going: bool,
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        // Prepare the Skyframe inputs.
        let build_configuration_key = self.create_build_configuration_key(build_options)?;
        // Skyframe-evaluate the configurations and throw errors if any.
        self.evaluate(build_configuration_key, keep_going)
    }

    /// The configuration of an already mapped key.
    pub fn get_configuration_for_key(
        &self,
        key: BuildConfigurationKey,
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        self.evaluate(key, true)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1894 SkyframeExecutor.createConfiguration
    /// Asks the Skyframe evaluator to build a `BuildConfigurationValue`.
    pub fn create_configuration(
        &self,
        event_handler: &dyn EventHandler,
        build_options: Arc<BuildOptions>,
        keep_going: bool,
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        let top_level_target_config = self.get_configuration(build_options, keep_going)?;
        // ErrorSensingEventHandler
        let sensing = ErrorSensing {
            delegate: event_handler,
            has_errors: std::sync::atomic::AtomicBool::new(false),
        };
        top_level_target_config.report_invalid_options(&sensing);
        if sensing.has_errors.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(InvalidConfigurationException::with_code(
                "Build options are invalid",
                "INVALID_BUILD_OPTIONS",
            ));
        }
        Ok(top_level_target_config)
    }
}

struct ErrorSensing<'a> {
    delegate: &'a dyn EventHandler,
    has_errors: std::sync::atomic::AtomicBool,
}

impl EventHandler for ErrorSensing<'_> {
    fn handle(&self, event: &Event) {
        if event.get_kind() == blaze_events::EventKind::Error {
            self.has_errors
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.delegate.handle(event);
    }
}

impl Hash for PlatformMappingValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.is_empty.hash(state)
    }
}
