//! The configuration SkyFunctions: BUILD_CONFIGURATION (`BuildConfigurationFunction`),
//! BUILD_CONFIGURATION_KEY (`BuildConfigurationKeyFunction` with `BuildConfigurationKeyProducer`),
//! BASELINE_OPTIONS (`BaselineOptionsFunction`), PLATFORM_MAPPING (`PlatformMappingFunction`,
//! `PlatformMappingValue`), and the top-level configuration creation of `SkyframeExecutor`
//! (`setBaselineConfiguration`, `createConfiguration`, `getConfiguration`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_optdefs::fragment::core_options::{CoreOptions, CoreOptionsOutputDirectoryNamingScheme};
use blaze_optdefs::fragment::platform_mapping_key::PlatformMappingKey;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::{Foreign, OptionsParsingException};
use blaze_skyframe::{
    ComputeError, ComputeResult, Environment, EvaluationContext, EvaluatorBuilder,
    FunctionHermeticity, Key, SkyError, SkyFunctionException, SkyFunctionName, SkyKey, SkyValue,
    Transience,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::{Path, RootedPath};
use indexmap::IndexMap;

use crate::build_configuration_key::{BuildConfigurationKey, BuildConfigurationKeyValueKey};
use crate::build_configuration_value::{BuildConfigurationValue, GlobalStateProvider};
use crate::build_options::{BuildOptions, ScopeDefinition, ScopeType, StarlarkOptionValue};
use crate::build_options_scope_value::{BuildOptionsScopeKey, BuildOptionsScopeValue};
use crate::fragment::{FragmentFactory, InvalidConfigurationException};
use crate::output_directories::ConfigDirectories;
use crate::parsed_flags::{ParsedFlagsKey, ParsedFlagsValue};
use crate::platform::{PlatformValueKey, SharedPlatformValueProvider};

impl Key for BuildConfigurationKey {
    type Value = BuildConfigurationValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_CONFIGURATION");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:111 SkyFunctions.BUILD_CONFIGURATION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the build options, workspace name, sibling repository layout and mnemonic.
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:341 BuildConfigurationValue.equals
    fn values_equal(old: &BuildConfigurationValue, new: &BuildConfigurationValue) -> bool {
        old == new
    }
}

impl Key for BuildConfigurationKeyValueKey {
    /// `BuildConfigurationKeyValue.buildConfigurationKey()`.
    type Value = BuildConfigurationKey;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_CONFIGURATION_KEY");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:113 SkyFunctions.BUILD_CONFIGURATION_KEY (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the `BuildConfigurationKey`, which compares its options.
    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:105 BuildConfigurationKeyValue.equals
    fn values_equal(old: &BuildConfigurationKey, new: &BuildConfigurationKey) -> bool {
        old == new
    }
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
            // String concatenation: `null` for no platform (checked against the oracle's
            // `dump --skyframe=deps`).
            self.new_platform
                .as_ref()
                .map_or_else(|| "null".to_owned(), |l| l.to_string())
        )
    }
}

impl Key for BaselineOptionsKey {
    /// `BaselineOptionsValue.toOptions()`.
    type Value = Arc<BuildOptions>;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BASELINE_OPTIONS");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:116 SkyFunctions.BASELINE_OPTIONS (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: a record of the `BuildOptions` (`BuildOptions.equals`, by checksum).
    // Java: com/google/devtools/build/lib/analysis/config/transitions/BaselineOptionsValue.java:50 BaselineOptionsValue
    fn values_equal(old: &Arc<BuildOptions>, new: &Arc<BuildOptions>) -> bool {
        old == new
    }
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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:131 SkyFunctions.PLATFORM_MAPPING (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: both mappings as maps, and the options classes (a server constant, not kept here). PORT: `flags_to_platforms` is compared in order; a reordered mappings file is unequal here, so Rust prunes less, never more.
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:162 PlatformMappingValue.equals
    fn values_equal(old: &PlatformMappingValue, new: &PlatformMappingValue) -> bool {
        old == new
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:50 PlatformMappingValue
/// Stores contents of a platforms/flags mapping file for transforming one `BuildOptions` into
/// another.
///
/// PORT: skipped, incremental: Java memoizes `computeMapping` per input (`mappingCache`).
#[derive(Debug, PartialEq, Eq)]
pub struct PlatformMappingValue {
    /// `ImmutableMap<Label, ParsedFlagsValue>` (insertion order).
    platforms_to_flags: IndexMap<Label, Arc<ParsedFlagsValue>>,
    /// `ImmutableMap<ParsedFlagsValue, Label>` (insertion order, unique keys).
    flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)>,
}

impl PlatformMappingValue {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:67 PlatformMappingValue.<init>
    /// Creates a new mapping value which will match on the given platforms (if a target platform
    /// is set on the key to be mapped), otherwise on the set of flags.
    pub fn new(
        platforms_to_flags: IndexMap<Label, Arc<ParsedFlagsValue>>,
        flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)>,
    ) -> Self {
        PlatformMappingValue {
            platforms_to_flags,
            flags_to_platforms,
        }
    }

    pub fn empty() -> Self {
        Self::new(IndexMap::new(), Vec::new())
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:103 PlatformMappingValue.map
    /// Maps one `BuildOptions` to another's `BuildConfigurationKey` by way of mappings provided
    /// in a file.
    ///
    /// 1. If a target platform is set on the original then mappings from platform to flags will
    ///    be applied.
    /// 2. If no target platform is set then mappings from flags to platforms will be applied.
    /// 3. If no matching flags to platforms mapping was found, the default target platform will
    ///    be used.
    pub fn map(
        &self,
        original_options: &BuildOptions,
    ) -> Result<BuildConfigurationKey, OptionsParsingException> {
        self.compute_mapping(original_options)
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:117 PlatformMappingValue.computeMapping
    fn compute_mapping(
        &self,
        original_options: &BuildOptions,
    ) -> Result<BuildConfigurationKey, OptionsParsingException> {
        if original_options.has_no_config() {
            // The empty configuration (produced by NoConfigTransition) is terminal: it'll never
            // change.
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
            let platforms = &platform_options.platforms;

            // Platform mapping only supports a single target platform, others are ignored.
            let target_platform = &platforms[0].0;
            let Some(parsed_flags) = self.platforms_to_flags.get(target_platform) else {
                // This can happen if the user has set the platform and any other flags that
                // would normally be mapped from it on the command line instead of relying on
                // the mapping.
                return Ok(BuildConfigurationKey::create(Arc::new(
                    original_options.clone(),
                )));
            };
            return Ok(parsed_flags.merge_with(original_options));
        }

        for (parsed_flags, platform_label) in &self.flags_to_platforms {
            if original_options.matches(parsed_flags.parsing_result())? {
                let mut modified_options = original_options.clone();
                modified_options
                    .get_mut::<PlatformOptions>()
                    .unwrap()
                    .platforms = vec![Foreign(platform_label.clone())];
                return Ok(BuildConfigurationKey::create(Arc::new(modified_options)));
            }
        }

        // No mapping found.
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
    /// `PlatformMappingException`: its message, which is the `toString()` of its cause (Java
    /// `super(cause)`), for example
    /// `com.google.devtools.build.lib.actions.MissingInputFileException: <message>`.
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
    pub baseline_configuration: PrecomputedOptions,
    /// `PrecomputedValue.BASELINE_EXEC_CONFIGURATION`.
    ///
    /// PORT: Java injects both values when the baseline is set
    /// (`SkyframeExecutor.setBaselineConfiguration`), loading the exec transition's .bzl with the
    /// same evaluator; Blaze's evaluator only exists once its functions (this client among them)
    /// are registered, so the client sets them after building the evaluator, and again when the
    /// build's options change (project files) before any configuration is computed.
    pub baseline_exec_configuration: PrecomputedOptions,
    pub platforms: SharedPlatformValueProvider,
    /// `RepositoryMappingValue.key(RepositoryName.MAIN)` (PlatformMappingFunction).
    pub main_repository_mapping: Arc<dyn MainRepositoryMappingLookup>,
    /// `FileValue.key(rootedPath)` (PlatformMappingFunction).
    pub file_lookup: Arc<dyn FileValueLookup>,
}

/// What `PlatformMappingFunction` reads of a `FileValue`.
#[derive(Clone, Debug)]
pub struct FileValueFacts {
    /// `fileValue.exists()`.
    pub exists: bool,
    /// `fileValue.isDirectory()`.
    pub is_directory: bool,
    /// `fileValue.realRootedPath(rootedPath).asPath()`.
    pub real_path: Path,
}

/// `env.getValue(FileValue.key(rootedPath))`.
///
/// PORT: the `FILE` key lives in blaze_sky_loading, which this crate cannot name; the executor
/// implements this lookup with it (I2 item X3, the pattern of [`MainRepositoryMappingLookup`]).
pub trait FileValueLookup: Send + Sync {
    /// The facts of the `FileValue` of `rooted_path`, or `None` when the value is missing or in
    /// error (Java's `getValue` returns null; the evaluator then reports the dependency's error).
    fn file_value(
        &self,
        env: &mut Environment<'_>,
        rooted_path: &RootedPath,
    ) -> Result<Option<FileValueFacts>, ComputeError>;
}

/// The lookup of the `blaze-config-dev` evaluator, which has no `FILE` function: a direct stat
/// (following symbolic links, as `FileValue` does). Not used by a Bazel command.
pub struct DirectStatFileValueLookup;

impl FileValueLookup for DirectStatFileValueLookup {
    fn file_value(
        &self,
        _env: &mut Environment<'_>,
        rooted_path: &RootedPath,
    ) -> Result<Option<FileValueFacts>, ComputeError> {
        let path = rooted_path.as_path();
        match path.stat_if_found(blaze_vfs::Symlinks::Follow) {
            Ok(stat) => Ok(Some(FileValueFacts {
                exists: stat.is_some(),
                is_directory: stat.is_some_and(|s| s.is_directory()),
                real_path: path,
            })),
            Err(e) => {
                blaze_util::unported!("FileFunction.compute", "stat of {}: {}", rooted_path, e)
            }
        }
    }
}

/// The baseline configuration, a dependency of the functions that read it.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:100 PrecomputedValue.BASELINE_CONFIGURATION
pub static BASELINE_CONFIGURATION: blaze_skyframe::precomputed::Precomputed<Arc<BuildOptions>> =
    blaze_skyframe::precomputed::Precomputed::new("baseline_configuration");

/// The baseline exec configuration, a dependency of the functions that read it.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:104 PrecomputedValue.BASELINE_EXEC_CONFIGURATION
pub static BASELINE_EXEC_CONFIGURATION: blaze_skyframe::precomputed::Precomputed<
    Arc<BuildOptions>,
> = blaze_skyframe::precomputed::Precomputed::new("baseline_exec_configuration");

/// A precomputed `BuildOptions` value of the configuration client (Java: a `PrecomputedValue`
/// injected into the graph): set before the first configuration is requested.
#[derive(Default)]
pub struct PrecomputedOptions(std::sync::RwLock<Option<Arc<BuildOptions>>>);

impl PrecomputedOptions {
    pub fn new(value: Option<Arc<BuildOptions>>) -> Self {
        PrecomputedOptions(std::sync::RwLock::new(value))
    }

    /// The value; panics if it was never set.
    pub fn get(&self) -> Arc<BuildOptions> {
        self.0
            .read()
            .unwrap()
            .clone()
            .expect("IllegalStateException: precomputed baseline options are not set")
    }

    pub fn is_set(&self) -> bool {
        self.0.read().unwrap().is_some()
    }

    pub fn set(&self, value: Arc<BuildOptions>) {
        *self.0.write().unwrap() = Some(value);
    }
}

fn client<'a>(env: &'a Environment<'_>) -> &'a ConfigurationEnvironment {
    env.client::<ConfigurationEnvironment>()
        .expect("ConfigurationEnvironment client")
}

fn persistent(e: impl Into<ConfigurationError>) -> ComputeError {
    SkyFunctionException::new(e.into(), Transience::Persistent).into()
}

/// `new PlatformMappingFunctionException(new MissingInputFileException(createFailureDetail(
/// message, code), Location.BUILTIN))`.
// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:353 PlatformMappingFunction.PlatformMappingFunctionException.<init>
fn platform_mapping_function_exception_missing_input_file(message: String) -> ComputeError {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingException.java:22 PlatformMappingException.<init>
    // `super(cause)`: the message is the cause's toString().
    persistent(ConfigurationError::PlatformMapping(format!(
        "com.google.devtools.build.lib.actions.MissingInputFileException: {message}"
    )))
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:70 PlatformMappingFunction.compute
fn compute_platform_mapping(
    key: &PlatformMappingSkyKey,
    env: &mut Environment<'_>,
) -> Result<Option<PlatformMappingValue>, ComputeError> {
    let workspace_relative_mapping_path = key.0.get_workspace_relative_mapping_path().clone();
    // Java: env.getValue(RepositoryMappingValue.key(RepositoryName.MAIN)); the key lives in
    // blaze_sky_loading, which this crate cannot name (see `MainRepositoryMappingLookup`).
    let lookup = client(env).main_repository_mapping.clone();
    let Some(main_repo_mapping) = lookup.main_repository_mapping(env)? else {
        return Ok(None);
    };
    let path_entries = client(env).package_path_roots.clone();
    for root in &path_entries {
        let rooted_mapping_path = RootedPath::to_rooted_path(
            blaze_vfs::Root::from_path(root.clone()),
            &workspace_relative_mapping_path,
        );
        let file_lookup = client(env).file_lookup.clone();
        let Some(file_value) = file_lookup.file_value(env, &rooted_mapping_path)? else {
            return Ok(None);
        };
        if !file_value.exists {
            continue;
        }
        if file_value.is_directory {
            return Err(platform_mapping_function_exception_missing_input_file(
                format!(
                    "--platform_mappings was set to '{}' relative to the top-level workspace '{}' but that path refers to a directory, not a file",
                    workspace_relative_mapping_path, root
                ),
            ));
        }
        let path = file_value.real_path;

        // Java: FileSystemUtils.readLines(path, UTF_8); malformed input is replaced.
        let bytes = match path.get_input_stream().and_then(|mut stream| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut bytes).map(|_| bytes)
        }) {
            Ok(bytes) => bytes,
            // Java: catch (IOException e) { throw new PlatformMappingFunctionException(e); }
            // (Transience.TRANSIENT). The message is the toString() of the Java IOException:
            // the native file system's FileAccessException for EACCES (recorded from the
            // oracle: "com.google.devtools.build.lib.vfs.FileAccessException: <path> (Permission
            // denied)").
            // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:357 PlatformMappingFunction.PlatformMappingFunctionException.<init>
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(SkyFunctionException::new(
                    ConfigurationError::PlatformMapping(format!(
                        "com.google.devtools.build.lib.vfs.FileAccessException: {path} (Permission denied)"
                    )),
                    Transience::Transient,
                )
                .into());
            }
            Err(e) => blaze_util::unported!(
                "PlatformMappingFunction.PlatformMappingFunctionException.<init>",
                "IOException ({:?}) reading the platform mappings file {}: {}",
                e.kind(),
                path,
                e
            ),
        };
        let text = blaze_util::string_encoding::java_new_string_utf8(&bytes);
        let lines = java_read_lines(&text);

        return match parse(env, &lines, &main_repo_mapping)? {
            Ok(None) => Ok(None),
            Ok(Some(parsed)) => Ok(Some(parsed.to_platform_mapping_value())),
            // Java: catch (PlatformMappingParsingException e) { throw new
            // PlatformMappingFunctionException(e); } (Transience.PERSISTENT)
            Err(e) => Err(persistent(ConfigurationError::PlatformMapping(
                e.to_string(),
            ))),
        };
    }

    if !key.0.was_explicitly_set_by_user() {
        // If no flag was passed and the default mapping file does not exist treat this as if
        // the mapping file was empty rather than an error.
        return Ok(Some(PlatformMappingValue::empty()));
    }
    Err(platform_mapping_function_exception_missing_input_file(
        format!(
            "--platform_mappings was set to '{}' but no such file exists relative to the package path roots, '[{}]'",
            workspace_relative_mapping_path,
            path_entries
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyFunction.java:42 BuildConfigurationKeyFunction.compute
/// Delegates to `BuildConfigurationKeyProducer` ([`build_configuration_key_producer`]).
fn compute_build_configuration_key(
    key: &BuildConfigurationKeyValueKey,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    build_configuration_key_producer(key.build_options(), key.producer_label(), env)
}

/// `BuildConfigurationKeyProducer`: the configuration key of one `BuildOptions`, with the
/// platform-based flags or the platform mapping and the scopes applied. The producer's steps run
/// in order in the caller's environment, so the caller depends on what they look up (PLATFORM,
/// PLATFORM_MAPPING, BUILD_OPTIONS_SCOPE, BASELINE_CONFIGURATION), as Java's state machine makes
/// the configured target that drives it depend on them. `Ok(None)` is "a Skyframe restart is
/// needed" (or the producer's "Error.": DONE after a missing value).
///
/// Errors are thrown as `BuildConfigurationKeyFunction` throws them (a persistent
/// `SkyFunctionException` whose cause is a [`ConfigurationError`], or the PLATFORM function's
/// `InvalidPlatformException`); the analysis producers (blaze_sky_analysis `transition_applier`)
/// catch them from the returned error and hand them to their sinks, as Java's
/// `BuildConfigurationKeyProducer.ResultSink` does.
// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:122 BuildConfigurationKeyProducer.step
pub fn build_configuration_key_producer(
    options: &Arc<BuildOptions>,
    label: Option<&Label>,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    // Short-circuit if there are no platform options.
    let Some(platform_options) = options.get::<PlatformOptions>() else {
        return find_build_options_scopes(options.clone(), label, env);
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
        // Java: com/google/devtools/build/lib/analysis/producers/PlatformProducer.java:61 PlatformProducer.step
        // `tasks.lookUp(key, InvalidPlatformException.class, OptionsParsingException.class,
        // this)`: an InvalidPlatformException goes to the sink (acceptPlatformFlagsError),
        // `Sink.checkErrors` rethrows it and BuildConfigurationKeyFunction throws it as its own
        // BuildConfigurationKeyFunctionException.
        // PORT: InvalidPlatformException is `blaze_analysis::toolchains::ToolchainException`,
        // which this crate cannot name; the PLATFORM provider recognizes it
        // (`PlatformValueProvider::catch_invalid_platform_exception`) and the same exception is
        // rethrown here.
        // An OptionsParsingException of PLATFORM (the PARSED_FLAGS error of the platform's
        // `flags`, which PlatformFunction does not catch) goes to the sink as well
        // (acceptOptionsParsingError) and is rethrown the same way.
        let platforms = client(env).platforms.clone();
        let target_platform_value =
            match env.get_value_or_throw_with(platform_key, |e: &SkyError| {
                platforms.catch_invalid_platform_exception(e).or_else(|| {
                    e.downcast_ref::<OptionsParsingException>()
                        .map(|_| e.clone())
                })
            })? {
                Ok(Some(target_platform_value)) => target_platform_value,
                // Java: BuildConfigurationKeyProducer.checkTargetPlatformFlags, "Error.": DONE.
                Ok(None) => return Ok(None),
                // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyFunction.java:72 BuildConfigurationKeyFunction.compute
                // `throw new BuildConfigurationKeyFunctionException(e)` (Transience.PERSISTENT).
                Err(invalid_platform_exception) => {
                    return Err(SkyFunctionException::from_cause(
                        invalid_platform_exception,
                        Transience::Persistent,
                    )
                    .into());
                }
            };
        // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:152 BuildConfigurationKeyProducer.checkTargetPlatformFlags
        match &target_platform_value.parsed_flags {
            Some(parsed_flags) => {
                let post_platform_processed_options =
                    parsed_flags.merge_with(options).get_options().clone();
                return find_build_options_scopes(post_platform_processed_options, label, env);
            }
            None => {}
        }
        merge_from_platform_mapping(options, label, env)
    } else {
        blaze_util::check_state!(target_platforms.is_empty());
        merge_from_platform_mapping(options, label, env)
    }
}

/// `FileSystemUtils.readLines`: lines end at `\n`, `\r` or `\r\n` (Guava `LineReader`); a
/// trailing line terminator does not start another line.
fn java_read_lines(text: &str) -> Vec<String> {
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
    lines
}

/// `env.getValue(RepositoryMappingValue.key(RepositoryName.MAIN))`, which
/// PlatformMappingFunction reads: the key lives in blaze_sky_loading, which this crate cannot
/// name, so the client supplies the lookup.
pub trait MainRepositoryMappingLookup: Send + Sync {
    /// The main repository's mapping, or `None` when the value is missing (Java returns null).
    fn main_repository_mapping(
        &self,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMapping>, ComputeError>;
}

/// The lookup of a client without repository mappings (the `blaze-config-dev` evaluator).
pub struct UnportedMainRepositoryMapping;

impl MainRepositoryMappingLookup for UnportedMainRepositoryMapping {
    fn main_repository_mapping(
        &self,
        _env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMapping>, ComputeError> {
        blaze_util::unported!(
            "RepositoryMappingFunction.compute",
            "main repository mapping in the blaze-config-dev evaluator"
        )
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingParsingException.java:23 PlatformMappingParsingException
/// Inner wrapper exception to work around the fact that `SkyFunctionException` cannot carry a
/// message of its own.
#[derive(Clone, Debug)]
pub struct PlatformMappingParsingException {
    /// `getMessage()`: the message given, or for `PlatformMappingParsingException(cause)` the
    /// cause's `toString()`.
    message: String,
}

impl PlatformMappingParsingException {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingParsingException.java:25 PlatformMappingParsingException.<init>
    fn new(message: String) -> Self {
        PlatformMappingParsingException { message }
    }

    /// `Throwable.getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingParsingException.java:29 PlatformMappingParsingException.<init>
    fn from_options_parsing_exception(cause: &OptionsParsingException) -> Self {
        PlatformMappingParsingException {
            message: format!(
                "com.google.devtools.common.options.OptionsParsingException: {}",
                cause.get_message()
            ),
        }
    }
}

impl fmt::Display for PlatformMappingParsingException {
    /// `Throwable.toString()`: the class name, then the message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "com.google.devtools.build.lib.skyframe.config.PlatformMappingParsingException: {}",
            self.message
        )
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:350 PlatformMappingFunction.parsingException
fn parsing_exception(message: String) -> PlatformMappingParsingException {
    PlatformMappingParsingException::new(message)
}

/// Simple data holder to make testing easier. Only for use internal to this file/tests thereof.
// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:362 PlatformMappingFunction.Mappings
pub struct Mappings {
    pub platforms_to_flags: IndexMap<Label, Arc<ParsedFlagsValue>>,
    pub flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)>,
}

impl Mappings {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:374 PlatformMappingFunction.Mappings.toPlatformMappingValue
    fn to_platform_mapping_value(self) -> PlatformMappingValue {
        PlatformMappingValue::new(self.platforms_to_flags, self.flags_to_platforms)
    }
}

/// The result of `parse`: `Ok(None)` where Java returns null (Skyframe dependencies missing).
type ParseResult<T> = Result<Result<Option<T>, PlatformMappingParsingException>, ComputeError>;

/// Java's `PeekingIterator` over the trimmed, non-empty, non-comment lines.
struct LineIterator {
    lines: Vec<String>,
    position: usize,
}

impl LineIterator {
    fn has_next(&self) -> bool {
        self.position < self.lines.len()
    }

    fn peek(&self) -> &str {
        &self.lines[self.position]
    }

    fn next(&mut self) -> String {
        self.position += 1;
        self.lines[self.position - 1].clone()
    }
}

/// Java `String.equalsIgnoreCase` with an ASCII literal.
fn equals_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:158 PlatformMappingFunction.parse
/// Parses the given lines, returns `None` if not all Skyframe deps are ready.
pub fn parse(
    env: &mut Environment<'_>,
    lines: &[String],
    main_repo_mapping: &RepositoryMapping,
) -> ParseResult<Mappings> {
    let mut it = LineIterator {
        lines: lines
            .iter()
            // String::trim removes the characters up to U+0020.
            .map(|line| line.trim_matches(|c: char| c <= ' ').to_string())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect(),
        position: 0,
    };

    if !it.has_next() {
        return Ok(Ok(Some(Mappings {
            platforms_to_flags: IndexMap::new(),
            flags_to_platforms: Vec::new(),
        })));
    }

    if !equals_ignore_case(it.peek(), "platforms:") && !equals_ignore_case(it.peek(), "flags:") {
        return Ok(Err(parsing_exception(format!(
            "Expected 'platforms:' or 'flags:' but got {}",
            it.peek()
        ))));
    }

    let mut platforms_to_flags = IndexMap::new();
    let mut flags_to_platforms = Vec::new();

    if equals_ignore_case(it.peek(), "platforms:") {
        it.next();
        match read_platforms_to_flags(&mut it, env, main_repo_mapping)? {
            Ok(Some(v)) => platforms_to_flags = v,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        }
    }

    if it.has_next() {
        let line = it.next();
        if !equals_ignore_case(&line, "flags:") {
            return Ok(Err(parsing_exception(format!(
                "Expected 'flags:' but got {}",
                line
            ))));
        }
        match read_flags_to_platforms(&mut it, env, main_repo_mapping)? {
            Ok(Some(v)) => flags_to_platforms = v,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        }
    }

    if it.has_next() {
        return Ok(Err(parsing_exception(format!(
            "Expected end of file but got {}",
            it.next()
        ))));
    }
    Ok(Ok(Some(Mappings {
        platforms_to_flags,
        flags_to_platforms,
    })))
}

/// `Label.RepoContext.rootPackage()` of `RepoContext.of(RepositoryName.MAIN, mapping)`.
// Java: com/google/devtools/build/lib/cmdline/Label.java:125 Label.RepoContext.rootPackage
fn main_root_package(main_repo_mapping: &RepositoryMapping) -> blaze_cmdline::LabelPackageContext {
    blaze_cmdline::LabelPackageContext::of(
        &blaze_cmdline::PackageIdentifier::create(
            blaze_cmdline::repository_name::MAIN.clone(),
            blaze_vfs::PathFragment::empty_fragment(),
        ),
        main_repo_mapping.clone(),
    )
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:211 PlatformMappingFunction.parseFlags
/// Converts a set of native and Starlark flag settings to a `ParsedFlagsValue`, or returns
/// `None` if not all Skyframe deps are ready.
fn parse_flags(
    raw_flags: Vec<String>,
    env: &mut Environment<'_>,
    main_repo_mapping: &RepositoryMapping,
) -> ParseResult<Arc<ParsedFlagsValue>> {
    let root_package = main_root_package(main_repo_mapping);
    // Passing an empty flagAliasMappings means platform mappings don't support flag aliases: if
    // a mapping sets --foo=bar but --foo is an alias for --//actual:flag, the mapping must be
    // updated to set --actual:flag=bar. Since this is a deprecated API we won't support mappings
    // here unless a compelling case demands it.
    let parsed_flags_key = ParsedFlagsKey::create(raw_flags, root_package, Vec::new());
    match env.get_value_or_throw_with(parsed_flags_key, |e: &SkyError| {
        e.downcast_ref::<OptionsParsingException>().cloned()
    })? {
        Ok(v) => Ok(Ok(v)),
        Err(e) => Ok(Err(
            PlatformMappingParsingException::from_options_parsing_exception(&e),
        )),
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:233 PlatformMappingFunction.readPlatformsToFlags
/// Returns a parsed `platform -> flags setting`, or `None` if not all Skyframe deps are ready.
fn read_platforms_to_flags(
    it: &mut LineIterator,
    env: &mut Environment<'_>,
    main_repo_mapping: &RepositoryMapping,
) -> ParseResult<IndexMap<Label, Arc<ParsedFlagsValue>>> {
    // ImmutableMap.Builder: a duplicate key fails buildOrThrow.
    let mut platforms_to_flags: Vec<(Label, Arc<ParsedFlagsValue>)> = Vec::new();
    let mut need_skyframe_deps = false;
    while it.has_next() && !equals_ignore_case(it.peek(), "flags:") {
        let platform = match read_platform(it, main_repo_mapping) {
            Ok(platform) => platform,
            Err(e) => return Ok(Err(e)),
        };
        let flags = match read_flags(it) {
            Ok(flags) => flags,
            Err(e) => return Ok(Err(e)),
        };
        match parse_flags(flags, env, main_repo_mapping)? {
            Ok(None) => need_skyframe_deps = true,
            Ok(Some(parsed_flags)) => platforms_to_flags.push((platform, parsed_flags)),
            Err(e) => return Ok(Err(e)),
        }
    }

    if need_skyframe_deps {
        return Ok(Ok(None));
    }

    let mut result = IndexMap::new();
    for (platform, flags) in platforms_to_flags {
        if result.insert(platform, flags).is_some() {
            return Ok(Err(parsing_exception(
                "Got duplicate platform entries but each platform key must be unique".to_string(),
            )));
        }
    }
    Ok(Ok(Some(result)))
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:264 PlatformMappingFunction.readFlagsToPlatforms
/// Returns a parsed `flags -> platform setting`, or `None` if not all Skyframe deps are ready.
fn read_flags_to_platforms(
    it: &mut LineIterator,
    env: &mut Environment<'_>,
    main_repo_mapping: &RepositoryMapping,
) -> ParseResult<Vec<(Arc<ParsedFlagsValue>, Label)>> {
    let mut flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)> = Vec::new();
    let mut need_skyframe_deps = false;
    while it.has_next() && it.peek().starts_with("--") {
        let flags = match read_flags(it) {
            Ok(flags) => flags,
            Err(e) => return Ok(Err(e)),
        };
        let platform = match read_platform(it, main_repo_mapping) {
            Ok(platform) => platform,
            Err(e) => return Ok(Err(e)),
        };

        match parse_flags(flags, env, main_repo_mapping)? {
            Ok(None) => need_skyframe_deps = true,
            Ok(Some(parsed_flags)) => flags_to_platforms.push((parsed_flags, platform)),
            Err(e) => return Ok(Err(e)),
        }
    }

    if need_skyframe_deps {
        return Ok(Ok(None));
    }

    // ImmutableMap.Builder.buildOrThrow fails on a duplicate key.
    for i in 0..flags_to_platforms.len() {
        for j in 0..i {
            if flags_to_platforms[i].0 == flags_to_platforms[j].0 {
                return Ok(Err(parsing_exception(
                    "Got duplicate flags entries but each flags key must be unique".to_string(),
                )));
            }
        }
    }
    Ok(Ok(Some(flags_to_platforms)))
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:296 PlatformMappingFunction.readPlatform
fn read_platform(
    it: &mut LineIterator,
    main_repo_mapping: &RepositoryMapping,
) -> Result<Label, PlatformMappingParsingException> {
    if !it.has_next() {
        return Err(parsing_exception(
            "Expected platform label but got end of file".to_string(),
        ));
    }

    let line = it.next();
    let main_repo_context = blaze_cmdline::LabelRepoContextImpl::of(
        blaze_cmdline::repository_name::MAIN.clone(),
        main_repo_mapping.clone(),
    );
    Label::parse_with_repo_context(&line, &main_repo_context)
        .map_err(|_| parsing_exception(format!("Expected platform label but got {}", line)))
}

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingFunction.java:311 PlatformMappingFunction.readFlags
fn read_flags(it: &mut LineIterator) -> Result<Vec<String>, PlatformMappingParsingException> {
    let mut flags = Vec::new();
    // Note: Short form flags are not supported.
    while it.has_next() && it.peek().starts_with("--") {
        flags.push(it.next());
    }
    if flags.is_empty() {
        return Err(parsing_exception(if it.has_next() {
            format!(
                "Expected a standard format flag (starting with --) but got {}",
                it.peek()
            )
        } else {
            "Expected a flag but got end of file".to_string()
        }));
    }
    Ok(flags)
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:209 BuildConfigurationKeyProducer.mergeFromPlatformMapping
fn merge_from_platform_mapping(
    options: &Arc<BuildOptions>,
    label: Option<&Label>,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    let mapping_key = PlatformMappingSkyKey(
        options
            .get::<PlatformOptions>()
            .unwrap()
            .platform_mapping_key
            .clone(),
    );
    // Java: tasks.lookUp(platformMappingKey, PlatformMappingException.class, this).
    let platform_mapping_value = match env.get_value_or_throw_with(mapping_key, |e| {
        e.downcast_ref::<ConfigurationError>()
            .filter(|c| matches!(c, ConfigurationError::PlatformMapping(_)))
            .cloned()
    })? {
        Ok(Some(v)) => v,
        // Java: BuildConfigurationKeyProducer.applyPlatformMapping, "Error.": DONE.
        Ok(None) => return Ok(None),
        // Java: sink.acceptPlatformMappingError(exception); Sink.checkErrors rethrows it as a
        // BuildConfigurationKeyFunctionException.
        Err(e) => return Err(persistent(e)),
    };
    // Java: BuildConfigurationKeyProducer.applyPlatformMapping
    match platform_mapping_value.map(options) {
        Ok(key) => find_build_options_scopes(key.get_options().clone(), label, env),
        Err(e) => Err(persistent(ConfigurationError::OptionsParsing(e))),
    }
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:170 BuildConfigurationKeyProducer.findBuildOptionsScopes
/// Performs a lookup for `BuildOptionsScopeValue` via `BuildOptionsScopeFunction` given
/// `post_platform_processed_options`. This is only done if there are any flag that has
/// `ScopeType.PROJECT` or its `ScopeType` is not yet resolved.
fn find_build_options_scopes(
    post_platform_processed_options: Arc<BuildOptions>,
    label: Option<&Label>,
    env: &mut Environment<'_>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    // including platform-based flags in skykey for scopes lookUp
    if post_platform_processed_options
        .get_starlark_options()
        .is_empty()
    {
        return possibly_apply_scopes(env, post_platform_processed_options, None, label);
    }

    // the list of flags that are either project scoped or their scopes are not yet resolved.
    // Lookup via BuildOptionsScopeFunction will be done for these flags
    let mut flags_with_incomplete_scope_info: Vec<Label> = Vec::new();
    for key in post_platform_processed_options
        .get_starlark_options()
        .keys()
    {
        let scope_type = post_platform_processed_options
            .get_scope_type_map()
            .get(key);
        // scope is null is applicable for cases where a transition applies starlark flags that
        // are not already part of the baseline configuration.
        if scope_type.is_none() || scope_type == Some(&ScopeType::PROJECT) {
            flags_with_incomplete_scope_info.push(key.clone());
        }
    }

    // if flagsWithIncompleteScopeInfo is empty, we do not need to do any further lookUp for the
    // ScopeType and ScopeDefinition
    if flags_with_incomplete_scope_info.is_empty() {
        return possibly_apply_scopes(env, post_platform_processed_options, None, label);
    }

    let build_options_scope_value_key = BuildOptionsScopeKey::create(
        post_platform_processed_options.clone(),
        flags_with_incomplete_scope_info,
    );
    // Java: tasks.lookUp(buildOptionsScopeValueKey, (Consumer<SkyValue>) this): an error of the
    // lookup is this node's error.
    let Some(build_options_scope_value) = env.get_value(build_options_scope_value_key)? else {
        return Ok(None);
    };
    possibly_apply_scopes(
        env,
        post_platform_processed_options,
        Some(build_options_scope_value),
        label,
    )
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:268 BuildConfigurationKeyProducer.possiblyApplyScopes
fn possibly_apply_scopes(
    env: &mut Environment<'_>,
    post_platform_processed_options: Arc<BuildOptions>,
    build_options_scope_value: Option<Arc<BuildOptionsScopeValue>>,
    label: Option<&Label>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    // This is not the same as null associated with Skyframe lookUp. This happens when scoping
    // logic is not enabled. This means the lookup via BuildOptionsScopesFunction was not
    // performed.
    let Some(build_options_scope_value) = build_options_scope_value else {
        return finish_configuration_key_processing(post_platform_processed_options);
    };
    if post_platform_processed_options
        .get_starlark_options()
        .is_empty()
    {
        return finish_configuration_key_processing(post_platform_processed_options);
    }

    let should_apply_scopes = build_options_scope_value
        .get_fully_resolved_scopes()
        .values()
        .any(|scope| scope.get_scope_type() == ScopeType::PROJECT);

    if !should_apply_scopes {
        return finish_configuration_key_processing(
            build_options_scope_value
                .get_resolved_build_options_with_scope_types()
                .clone(),
        );
    }

    // TODO: b/390669368 - The same performance issue still exists if we reach this point.
    // Java: tasks.lookUp(PrecomputedValue.BASELINE_CONFIGURATION.getKey(), ...)
    let Some(baseline_configuration) = BASELINE_CONFIGURATION.get(env)? else {
        return Ok(None);
    };
    let baseline_configuration: Arc<BuildOptions> = (*baseline_configuration).clone();
    // BuildConfigurationKeyFunction passes a null label; the analysis producers
    // (BuildConfigurationKeyMapProducer) pass the target's label, which Blaze carries in the
    // request (`BuildConfigurationKeyValueKey::producer_label`).
    apply_scopes(&build_options_scope_value, &baseline_configuration, label)
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:292 BuildConfigurationKeyProducer.applyScopes
fn apply_scopes(
    build_options_scope_value: &BuildOptionsScopeValue,
    baseline_configuration: &Arc<BuildOptions>,
    label: Option<&Label>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    let resolved = build_options_scope_value.get_resolved_build_options_with_scope_types();
    // baselineConfiguration.getStarlarkOptions().equals(resolved.getStarlarkOptions())
    let baseline_starlark_options = baseline_configuration.get_starlark_options();
    let resolved_starlark_options = resolved.get_starlark_options();
    let starlark_options_equal = baseline_starlark_options.len() == resolved_starlark_options.len()
        && resolved_starlark_options.iter().all(|(key, value)| {
            baseline_starlark_options
                .get(key)
                .is_some_and(|baseline_value| baseline_value.java_equals(value))
        });
    let final_build_options = if starlark_options_equal {
        resolved.clone()
    } else {
        reset_flags(build_options_scope_value, baseline_configuration, label)
    };
    finish_configuration_key_processing(final_build_options)
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:301 BuildConfigurationKeyProducer.finishConfigurationKeyProcessing
fn finish_configuration_key_processing(
    final_build_options: Arc<BuildOptions>,
) -> Result<Option<BuildConfigurationKey>, ComputeError> {
    Ok(Some(BuildConfigurationKey::create(final_build_options)))
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:320 BuildConfigurationKeyProducer.resetFlags
/// If a flag is considered to be out of scope, resetFlags does either of the following:
///
/// - If the flag is not present in the baseline configuration, remove the flag from the
///   `BuildOptions`.
/// - If the flag is present in the baseline configuration, set the flag to the baseline value.
///   This is to ensure that we do not trigger an additional ST-<hash>, which defeats the purpose
///   of scoping.
///
/// This method returns the final `BuildOptions` after scoping is applied and the object only has
/// the `Scope.ScopeType` information for all starlark flags.
fn reset_flags(
    build_options_scope_value: &BuildOptionsScopeValue,
    baseline_configuration: &Arc<BuildOptions>,
    label: Option<&Label>,
) -> Arc<BuildOptions> {
    let transitioned_options_with_scope_type =
        build_options_scope_value.get_resolved_build_options_with_scope_types();
    // If there are no scopes, short circuit.
    if build_options_scope_value
        .get_fully_resolved_scopes()
        .is_empty()
    {
        return transitioned_options_with_scope_type.clone();
    }

    let mut flags_removed = false;
    let mut flags_reset_to_baseline = false;
    let mut options_with_scope_types_builder = transitioned_options_with_scope_type.to_builder();
    for (flag_label, flag_value) in transitioned_options_with_scope_type.get_starlark_options() {
        let scope = build_options_scope_value
            .get_fully_resolved_scopes()
            .get(flag_label);
        match scope {
            None => {
                // Verify.verify
                if transitioned_options_with_scope_type
                    .get_scope_type_map()
                    .get(flag_label)
                    == Some(&ScopeType::PROJECT)
                {
                    panic!("VerifyException");
                }
            }
            Some(scope) if scope.get_scope_type() == ScopeType::PROJECT => {
                let baseline_value: Option<&StarlarkOptionValue> = baseline_configuration
                    .get_starlark_options()
                    .get(flag_label);
                // PORT: Java compares the two objects by reference (`flagValue !=
                // baselineValue`). Values have no identity here; structural equality gives the
                // same resulting options: an equal value that is a different object is "reset"
                // to an equal value.
                if baseline_value != Some(flag_value)
                    && !is_in_scope(label, scope.get_scope_definition())
                {
                    match baseline_value {
                        None => {
                            options_with_scope_types_builder.remove_starlark_option(flag_label);
                            flags_removed = true;
                        }
                        Some(baseline_value) => {
                            options_with_scope_types_builder
                                .add_starlark_option(flag_label.clone(), baseline_value.clone());
                            flags_reset_to_baseline = true;
                        }
                    }
                }
            }
            Some(_) => {}
        }
    }

    if !flags_removed && !flags_reset_to_baseline {
        return transitioned_options_with_scope_type.clone();
    }

    let scoped_build_options = options_with_scope_types_builder.build();
    if scoped_build_options == **baseline_configuration {
        return baseline_configuration.clone();
    }

    Arc::new(scoped_build_options)
}

// Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:374 BuildConfigurationKeyProducer.isInScope
fn is_in_scope(label: Option<&Label>, scope_definition: Option<&ScopeDefinition>) -> bool {
    // A null scopeDefinition means the flag's package has no PROJECT.scl file. Treat the target
    // as not in scope so the flag resets to its baseline value.
    // Also, if the label is null, we are evaluating a configuration without a target, so we also
    // treat it as out of scope.
    let (Some(label), Some(scope_definition)) = (label, scope_definition) else {
        return false;
    };
    for path in scope_definition.get_owned_code_paths() {
        if label.get_canonical_form().starts_with(path.as_str()) {
            return true;
        }
    }
    false
}

// Java: com/google/devtools/build/lib/skyframe/config/BaselineOptionsFunction.java:43 BaselineOptionsFunction.compute
fn compute_baseline_options(
    key: &BaselineOptionsKey,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<BuildOptions>>, ComputeError> {
    let raw_baseline_options = if key.after_exec_transition {
        // Use the precomputed baseline exec
        BASELINE_EXEC_CONFIGURATION.get(env)?
    } else {
        // Use the standard baseline
        BASELINE_CONFIGURATION.get(env)?
    };
    let Some(raw_baseline_options) = raw_baseline_options else {
        return Ok(None);
    };
    let raw_baseline_options: Arc<BuildOptions> = (*raw_baseline_options).clone();

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

// Java: com/google/devtools/build/lib/skyframe/config/BaselineOptionsFunction.java:98 BaselineOptionsFunction.mapBuildOptions
fn map_build_options(
    env: &mut Environment<'_>,
    raw_baseline_options: Arc<BuildOptions>,
) -> Result<Option<Arc<BuildOptions>>, ComputeError> {
    let bckvk = BuildConfigurationKeyValueKey::create(raw_baseline_options);
    // Java: env.getValueOrThrow(bckvk, OptionsParsingException.class,
    // PlatformMappingException.class, InvalidPlatformException.class).
    // PORT: InvalidPlatformException is recognized by the PLATFORM provider (see
    // compute_build_configuration_key).
    let platforms = client(env).platforms.clone();
    match env.get_value_or_throw_with(bckvk, |e: &SkyError| {
        if e.downcast_ref::<ConfigurationError>().is_some_and(|c| {
            matches!(
                c,
                ConfigurationError::OptionsParsing(_) | ConfigurationError::PlatformMapping(_)
            )
        }) {
            return Some(e.clone());
        }
        platforms.catch_invalid_platform_exception(e)
    })? {
        Ok(Some(v)) => Ok(Some(v.get_options().clone())),
        Ok(None) => Ok(None),
        // Java: throw new BaselineOptionsFunctionException(e);
        Err(e) => Err(SkyFunctionException::from_cause(e, Transience::Persistent).into()),
    }
}

// Java: com/google/devtools/build/lib/analysis/config/ConfigurationValueEvent.java:21 ConfigurationValueEvent
/// Posted when a `BuildConfigurationValue` is created.
///
/// PORT: Java keeps the class in `analysis/config`; it is here, next to its one poster. Nothing
/// in Bazel subscribes to it (the build event protocol gets its `BuildConfigurationEvent`s from
/// `BuildEvent.getConfigurations()` of the target events).
#[derive(Clone, Debug)]
pub struct ConfigurationValueEvent {
    configuration: Arc<BuildConfigurationValue>,
}

impl ConfigurationValueEvent {
    // Java: com/google/devtools/build/lib/analysis/config/ConfigurationValueEvent.java:26 ConfigurationValueEvent.create
    pub fn create(configuration: Arc<BuildConfigurationValue>) -> Self {
        ConfigurationValueEvent { configuration }
    }

    pub fn configuration(&self) -> &Arc<BuildConfigurationValue> {
        &self.configuration
    }
}

// Java: `storeForReplay()` is not overridden: the event is not stored in the node.
impl Postable for ConfigurationValueEvent {}

/// The BUILD_CONFIGURATION SkyFunction.
///
/// PORT: not a `typed_function`: the value is shared between the node and the
/// `ConfigurationValueEvent` the function posts, as in Java.
// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:57 BuildConfigurationFunction.compute
fn build_configuration_function(key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
    let key = key
        .downcast::<BuildConfigurationKey>()
        .expect("key type matches function name");
    Ok(compute_build_configuration(key, env)?.map(|v| v as SkyValue))
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:57 BuildConfigurationFunction.compute
fn compute_build_configuration(
    key: &BuildConfigurationKey,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<BuildConfigurationValue>>, ComputeError> {
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
        Ok(v) => {
            let configuration_value = Arc::new(v);
            env.listener()
                .post(Arc::new(ConfigurationValueEvent::create(
                    configuration_value.clone(),
                )));
            Ok(Some(configuration_value))
        }
        Err(e) => Err(persistent(e)),
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:96 BuildConfigurationFunction.getPlatformCpu
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

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationFunction.java:133 BuildConfigurationFunction.getBaselineOptions
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

    // Java: env.getValueOrThrow(BaselineOptionsValue.key(applyExecTransitionToBaseline,
    // newPlatform), StarlarkExecTransitionLoadingException.class): any other error of the
    // dependency makes it missing.
    match env.get_value_or_throw_with(
        BaselineOptionsKey {
            after_exec_transition: apply_exec_transition_to_baseline,
            new_platform,
        },
        |e| match e.downcast_ref::<ConfigurationError>() {
            Some(ConfigurationError::StarlarkExecTransitionLoading(e)) => Some(e.clone()),
            _ => None,
        },
    )? {
        Ok(Some(v)) => Ok(Some(Some(v.as_ref().clone()))),
        Ok(None) => Ok(None),
        // Java: throw new BuildConfigurationFunctionException(new
        // InvalidConfigurationException(e));
        Err(e) => Err(persistent(InvalidConfigurationException::new(e.0.clone()))),
    }
}

/// Registers the configuration functions on an evaluator builder. The client must be a
/// [`ConfigurationEnvironment`].
pub fn register_configuration_functions(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .function(
            <BuildConfigurationKey as Key>::FUNCTION,
            build_configuration_function,
        )
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
/// PORT: superseded by the `SkyframeExecutor` port in `blaze_sky_top`
/// (`skyframe_executor.rs`), which registers [`register_configuration_functions`] in the shared
/// evaluator. This one computes configurations in an evaluator of its own, created once the
/// baseline configuration is known (Java injects the baseline as precomputed values into the
/// shared graph); its only user left is the `blaze-config-dev` binary
/// (`top_level::create_top_level_configuration`).
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
            baseline_configuration: PrecomputedOptions::new(Some(build_options)),
            baseline_exec_configuration: PrecomputedOptions::new(Some(Arc::new(baseline_exec))),
            platforms: request.platforms,
            main_repository_mapping: Arc::new(UnportedMainRepositoryMapping),
            file_lookup: Arc::new(DirectStatFileValueLookup),
        };
        let client = Arc::new(client);
        let mut builder = register_configuration_functions(EvaluatorBuilder::new());
        builder = BASELINE_CONFIGURATION.set(builder, client.baseline_configuration.get());
        builder =
            BASELINE_EXEC_CONFIGURATION.set(builder, client.baseline_exec_configuration.get());
        let evaluator = builder.client(client.clone()).build();
        Ok(ConfigurationSkyframe {
            evaluator,
            client,
            event_handler,
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1544 SkyframeExecutor.adjustForExec
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
        self.client().baseline_exec_configuration.get()
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

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2204 SkyframeExecutor.createBuildConfigurationKey
    pub fn create_build_configuration_key(
        &self,
        build_options: Arc<BuildOptions>,
    ) -> Result<BuildConfigurationKey, InvalidConfigurationException> {
        Ok(self
            .evaluate(BuildConfigurationKeyValueKey::create(build_options), false)?
            .as_ref()
            .clone())
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2108 SkyframeExecutor.getConfiguration
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
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingValue.java:174 PlatformMappingValue.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (flags, platform) in &self.flags_to_platforms {
            flags.hash(state);
            platform.hash(state);
        }
        for (platform, flags) in &self.platforms_to_flags {
            platform.hash(state);
            flags.hash(state);
        }
    }
}
