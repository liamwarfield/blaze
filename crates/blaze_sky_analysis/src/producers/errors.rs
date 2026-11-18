//! The error and state types the configured-target producers share: `skyframe/
//! ConfiguredValueCreationException`, `analysis/config/DependencyEvaluationException`,
//! `analysis/InconsistentNullConfigException`, `analysis/TransitiveDependencyState`,
//! `analysis/TargetAndConfiguration`, `causes/{AnalysisFailedCause,LoadingFailedCause}` and
//! `analysis/producers/{DependencyError,DependencyContextError,MissingEdgeError}`.

use std::fmt;
use std::sync::Arc;

use blaze_analysis::config::starlark_transition::TransitionException;
use blaze_analysis::config::transitions::TransitionCreationException;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_analysis::rule::action_conflict_exception::ActionConflictException;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::exceptions::{
    DetailedExitCode, FailureCode, NoSuchPackageKind, NoSuchThingException,
};
use blaze_packages::package::PackageMetadata;
use blaze_packages::target::Target;
use blaze_packages::target::target_utils::{
    format_missing_edge_with_attribute, get_location_maybe,
};
use blaze_starlark::Location;

/// `blaze_starlark::Location` → event location.
pub fn event_location(location: Option<&Location>) -> Option<blaze_events::Location> {
    location.map(blaze_analysis::config::transitions::to_event_location)
}

/// Java `DetailedExitCode.of(FailureDetail{message, Analysis.CONFIGURED_VALUE_CREATION_FAILED})`.
// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:341 TargetAndConfigurationProducer.createDetailedExitCode
pub fn create_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode {
        message: message.to_owned(),
        code: FailureCode::Other("Analysis.CONFIGURED_VALUE_CREATION_FAILED".to_owned()),
    }
}

/// Java `ConfigurationId` of a configuration (`configurationIdMessage`): its checksum, or
/// `"none"` for the null configuration.
// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:333 TargetAndConfigurationProducer.configurationIdMessage
pub fn configuration_id_message(options_checksum: Option<&str>) -> String {
    match options_checksum {
        None => "none".to_owned(),
        Some(c) => c.to_owned(),
    }
}

/// Java `BuildEventIdUtil.configurationId(BuildConfigurationValue)`.
pub fn configuration_id(configuration: Option<&BuildConfigurationValue>) -> Option<String> {
    Some(configuration_id_message(
        configuration.map(|c| c.checksum()),
    ))
}

/// Java `BuildEventIdUtil.configurationId(BuildConfigurationKey)`.
pub fn configuration_id_of_key(key: Option<&BuildConfigurationKey>) -> Option<String> {
    Some(configuration_id_message(
        key.map(|k| k.get_options_checksum()),
    ))
}

/// A root cause of an analysis failure (Java `Cause`: `AnalysisFailedCause`,
/// `LoadingFailedCause`).
// Java: com/google/devtools/build/lib/causes/Cause.java:24 Cause
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    // Java: com/google/devtools/build/lib/causes/AnalysisFailedCause.java:30 AnalysisFailedCause
    AnalysisFailed {
        label: Label,
        configuration: String,
        detailed_exit_code: DetailedExitCode,
    },
    // Java: com/google/devtools/build/lib/causes/LoadingFailedCause.java:27 LoadingFailedCause
    LoadingFailed {
        label: Label,
        detailed_exit_code: DetailedExitCode,
    },
}

impl Cause {
    pub fn get_label(&self) -> &Label {
        match self {
            Cause::AnalysisFailed { label, .. } | Cause::LoadingFailed { label, .. } => label,
        }
    }

    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        match self {
            Cause::AnalysisFailed {
                detailed_exit_code, ..
            }
            | Cause::LoadingFailed {
                detailed_exit_code, ..
            } => detailed_exit_code,
        }
    }
}

/// Java `DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie(first, second)`.
///
/// PORT: the comparator orders by numeric exit code. Every code analysis produces here
/// (`Analysis.*`, `PackageLoading.*`) maps to exit code 1 (BUILD_FAILURE), so the first non-null
/// code always wins the tie.
// Java: com/google/devtools/build/lib/util/DetailedExitCodeComparator.java:44 DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie
pub fn choose_more_important_with_first_if_tie(
    first: Option<DetailedExitCode>,
    second: Option<DetailedExitCode>,
) -> Option<DetailedExitCode> {
    match (first, second) {
        (None, second) => second,
        (first, None) => first,
        (Some(first), Some(_)) => Some(first),
    }
}

/// An exception indicating that there was a problem during the construction of a
/// `ConfiguredTargetValue`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredValueCreationException.java:36 ConfiguredValueCreationException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ConfiguredValueCreationException {
    location: Option<Location>,
    message: String,
    /// Java `BuildEventId configuration` (`None` is Java null).
    configuration: Option<String>,
    root_causes: NestedSet<Cause>,
    detailed_exit_code: DetailedExitCode,
}

impl ConfiguredValueCreationException {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredValueCreationException.java:44 ConfiguredValueCreationException.<init>
    pub fn new(
        location: Option<Location>,
        message: impl Into<String>,
        label: &Label,
        configuration: Option<String>,
        root_causes: Option<NestedSet<Cause>>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        let message = message.into();
        let exit_code = detailed_exit_code.unwrap_or_else(|| create_detailed_exit_code(&message));
        let root_causes = root_causes.unwrap_or_else(|| {
            NestedSetBuilder::create(
                Order::STABLE_ORDER,
                [Cause::AnalysisFailed {
                    label: label.clone(),
                    configuration: configuration.clone().unwrap_or_else(|| "none".to_owned()),
                    detailed_exit_code: exit_code.clone(),
                }],
            )
        });
        ConfiguredValueCreationException {
            location,
            message,
            configuration,
            root_causes,
            detailed_exit_code: exit_code,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredValueCreationException.java:66 ConfiguredValueCreationException.<init>
    pub fn for_target_full(
        target: &Target,
        configuration: Option<String>,
        message: impl Into<String>,
        root_causes: Option<NestedSet<Cause>>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        Self::new(
            Some(target.get_location()),
            message,
            target.get_label(),
            configuration,
            root_causes,
            detailed_exit_code,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredValueCreationException.java:81 ConfiguredValueCreationException.<init>
    pub fn for_target(target: &Target, message: impl Into<String>) -> Self {
        Self::for_target_full(target, None, message, None, None)
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Clones this exception with its `message` field removed (what a `ReportedException`
    /// carries: the error was already reported to the user, so callers like `BuildTool` must not
    /// report it again).
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetEvaluationExceptions.java:34 ReportedException.withoutMessage
    pub fn without_message(&self) -> ConfiguredValueCreationException {
        ConfiguredValueCreationException {
            location: self.location.clone(),
            message: String::new(),
            configuration: self.configuration.clone(),
            root_causes: self.root_causes.clone(),
            detailed_exit_code: self.detailed_exit_code.clone(),
        }
    }

    pub fn get_location(&self) -> Option<&Location> {
        self.location.as_ref()
    }

    pub fn get_root_causes(&self) -> &NestedSet<Cause> {
        &self.root_causes
    }

    pub fn get_configuration(&self) -> Option<&String> {
        self.configuration.as_ref()
    }

    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

/// Exception that signals an error during the evaluation of a configured target dependency.
// Java: com/google/devtools/build/lib/analysis/config/DependencyEvaluationException.java:36 DependencyEvaluationException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct DependencyEvaluationException {
    message: String,
    /// The `ConfiguredValueCreationException` cause, if that is the cause.
    cause: Option<ConfiguredValueCreationException>,
    detailed_exit_code: Option<DetailedExitCode>,
    location: Option<Location>,
    dep_reported_own_error: bool,
}

impl DependencyEvaluationException {
    // Java: com/google/devtools/build/lib/analysis/config/DependencyEvaluationException.java:54 DependencyEvaluationException.<init>
    pub fn of_creation(
        cause: ConfiguredValueCreationException,
        dep_reported_own_error: bool,
    ) -> Self {
        DependencyEvaluationException {
            message: cause.get_message().to_owned(),
            detailed_exit_code: Some(cause.get_detailed_exit_code().clone()),
            location: cause.get_location().cloned(),
            cause: Some(cause),
            dep_reported_own_error,
        }
    }

    /// Java `DependencyEvaluationException(EvalException, Location)`.
    // Java: com/google/devtools/build/lib/analysis/config/DependencyEvaluationException.java:66 DependencyEvaluationException.<init>
    pub fn of_eval(message: String, location: Option<Location>) -> Self {
        DependencyEvaluationException {
            message,
            cause: None,
            detailed_exit_code: None,
            location,
            dep_reported_own_error: false,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_detailed_exit_code(&self) -> Option<&DetailedExitCode> {
        self.detailed_exit_code.as_ref()
    }

    pub fn get_location(&self) -> Option<&Location> {
        self.location.as_ref()
    }

    pub fn dep_reported_own_error(&self) -> bool {
        self.dep_reported_own_error
    }

    /// Java `getCause() instanceof ConfiguredValueCreationException`.
    pub fn get_creation_cause(&self) -> Option<&ConfiguredValueCreationException> {
        self.cause.as_ref()
    }
}

/// Thrown when a target that requires a configuration is requested with the null
/// configuration.
// Java: com/google/devtools/build/lib/analysis/InconsistentNullConfigException.java:17 InconsistentNullConfigException
#[derive(Clone, Debug, thiserror::Error)]
#[error("InconsistentNullConfigException")]
pub struct InconsistentNullConfigException;

/// The exceptions `ConfiguredTargetFunction.compute` throws (the cause of its
/// `SkyFunctionException`).
///
/// PORT: Java throws `ReportedException`/`UnreportedException` (wrapping a
/// `ConfiguredValueCreationException` or an `ActionConflictException`) and `DependencyException`
/// (wrapping a `NoSuchThingException` or an `InconsistentNullConfigException`); parents catch
/// the wrapped exception classes. One enum carries all of them.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ConfiguredTargetFunctionException {
    /// `ReportedException(ConfiguredValueCreationException)`.
    #[error("{0}")]
    Reported(ConfiguredValueCreationException),
    /// `UnreportedException(ConfiguredValueCreationException)`.
    #[error("{0}")]
    Unreported(ConfiguredValueCreationException),
    /// `UnreportedException(ActionConflictException)`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:405 ConfiguredTargetFunction.compute
    #[error("{0}")]
    ActionConflict(ActionConflictException),
    /// `DependencyException(NoSuchThingException)`.
    #[error("{0}")]
    NoSuchThing(NoSuchThingException),
    /// `DependencyException(InconsistentNullConfigException)`.
    #[error("{0}")]
    InconsistentNullConfig(InconsistentNullConfigException),
}

impl ConfiguredTargetFunctionException {
    /// The `ConfiguredValueCreationException` a parent catches, if this is one.
    pub fn as_configured_value_creation(&self) -> Option<&ConfiguredValueCreationException> {
        match self {
            ConfiguredTargetFunctionException::Reported(e)
            | ConfiguredTargetFunctionException::Unreported(e) => Some(e),
            _ => None,
        }
    }

    /// The `ActionConflictException` a parent or the error processor catches (Java
    /// `errorInfo.getException() instanceof ActionConflictException`), if this is one.
    pub fn as_action_conflict(&self) -> Option<&ActionConflictException> {
        match self {
            ConfiguredTargetFunctionException::ActionConflict(e) => Some(e),
            _ => None,
        }
    }
}

/// A `ConfiguredTargetFunctionException::ActionConflict` for the tests of the callers that catch
/// it: an `ActionConflictException` carries the artifact and the attempted action, here a
/// `FileWriteAction` of a dummy owner (`ActionsTestUtil.NULL_ACTION_OWNER`).
#[cfg(test)]
pub(crate) fn action_conflict_for_testing(message: &str) -> ConfiguredTargetFunctionException {
    use blaze_actions::actions::platform_info::empty_platform_info;
    use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
    use blaze_actions::{
        ActionLookupKeyRef, ActionOwner, ActionRef, Artifact, ArtifactRoot, RootType,
    };
    use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};

    let label = Label::parse_canonical_unchecked("//null/action:owner");
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let bin_root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let artifact = Artifact::create_derived_artifact(
        bin_root.clone(),
        bin_root
            .get_exec_path()
            .get_relative(&PathFragment::create("null/action/out.txt")),
        ActionLookupKeyRef::new(
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(None)
                .build(),
        ),
    );
    let owner = ActionOwner::create_dummy(
        Some(label),
        blaze_events::location::Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        indexmap::IndexMap::new(),
    );
    let action: ActionRef = Arc::new(FileWriteAction::create(
        owner,
        artifact.clone(),
        "",
        false,
        Compression::DISALLOW,
    ));
    ConfiguredTargetFunctionException::ActionConflict(ActionConflictException::create_with_message(
        &artifact,
        &action,
        message.to_owned(),
        /* is_prefix_conflict= */ false,
    ))
}

/// What a lookup of a `CONFIGURED_TARGET` dependency produced (Java's
/// `ValueOrException3Sink<ConfiguredValueCreationException, NoSuchThingException,
/// InconsistentNullConfigException>`).
#[derive(Clone, Debug)]
pub enum ConfiguredTargetLookupError {
    Creation(ConfiguredValueCreationException),
    NoSuchThing(NoSuchThingException),
    InconsistentNullConfig(InconsistentNullConfigException),
}

/// The catch closure of a `CONFIGURED_TARGET` lookup (Java `getValueOrThrow(key,
/// ConfiguredValueCreationException.class, NoSuchThingException.class,
/// InconsistentNullConfigException.class)`).
pub fn catch_configured_target_error(
    e: &blaze_skyframe::SkyError,
) -> Option<ConfiguredTargetLookupError> {
    let e = e.downcast_ref::<ConfiguredTargetFunctionException>()?;
    match e {
        ConfiguredTargetFunctionException::Reported(c)
        | ConfiguredTargetFunctionException::Unreported(c) => {
            Some(ConfiguredTargetLookupError::Creation(c.clone()))
        }
        ConfiguredTargetFunctionException::NoSuchThing(n) => {
            Some(ConfiguredTargetLookupError::NoSuchThing(n.clone()))
        }
        ConfiguredTargetFunctionException::InconsistentNullConfig(i) => Some(
            ConfiguredTargetLookupError::InconsistentNullConfig(i.clone()),
        ),
        ConfiguredTargetFunctionException::ActionConflict(_) => None,
    }
}

/// A target and its configuration (`None` for the null configuration).
// Java: com/google/devtools/build/lib/analysis/TargetAndConfiguration.java:30 TargetAndConfiguration
#[derive(Clone, Debug)]
pub struct TargetAndConfiguration {
    target: Target,
    configuration: Option<Arc<BuildConfigurationValue>>,
}

impl TargetAndConfiguration {
    pub fn new(target: Target, configuration: Option<Arc<BuildConfigurationValue>>) -> Self {
        TargetAndConfiguration {
            target,
            configuration,
        }
    }

    pub fn get_target(&self) -> &Target {
        &self.target
    }

    pub fn get_label(&self) -> &Label {
        self.target.get_label()
    }

    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }

    /// Java `getConfigurationKey()`.
    pub fn get_configuration_key(&self) -> Option<BuildConfigurationKey> {
        self.configuration.as_ref().map(|c| c.get_key())
    }
}

/// Transitive state of a configured target's dependency resolution (root causes, packages).
// Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:46 TransitiveDependencyState
pub struct TransitiveDependencyState {
    transitive_root_causes: NestedSetBuilder<Cause>,
    /// Non-null when transitive packages are tracked.
    package_collector: Option<PackageCollector>,
}

/// Collects package metadata of dependencies to be unified in a NestedSet, with bookkeeping so
/// the result is deterministic (Skyframe work may complete in arbitrary order).
// Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:166 TransitiveDependencyState.PackageCollector
struct PackageCollector {
    /// Keeps packages that were added directly as a list. These will be sorted.
    packages: Vec<Arc<PackageMetadata>>,
    /// Stores transitive package metadata of `ConfiguredTargetValue`s, in
    /// `CONFIGURED_TARGET_KEY_ORDERING`.
    configured_target_packages: Vec<(ConfiguredTargetKey, NestedSet<RepoMappingPackageMetadata>)>,
}

type RepoMappingPackageMetadata =
    blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata;

/// Java `CONFIGURED_TARGET_KEY_ORDERING`: label, then execution platform label (nulls first),
/// then configuration options checksum (nulls first).
// Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:199 TransitiveDependencyState.CONFIGURED_TARGET_KEY_ORDERING
fn configured_target_key_ordering(
    a: &ConfiguredTargetKey,
    b: &ConfiguredTargetKey,
) -> std::cmp::Ordering {
    a.get_label()
        .cmp(b.get_label())
        .then_with(|| {
            a.get_execution_platform_label()
                .cmp(&b.get_execution_platform_label())
        })
        .then_with(|| {
            a.get_configuration_key()
                .map(|k| k.get_options_checksum())
                .cmp(&b.get_configuration_key().map(|k| k.get_options_checksum()))
        })
}

impl PackageCollector {
    /// Constructs the deterministically ordered result. It's safe to call this multiple times.
    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:186 TransitiveDependencyState.PackageCollector.buildSet
    fn build_set(&self) -> NestedSet<RepoMappingPackageMetadata> {
        let mut result = NestedSetBuilder::stable_order();
        let mut packages = self.packages.clone();
        packages.sort_by(|a, b| a.package_identifier.cmp(&b.package_identifier));
        result.add_all(packages.iter().map(|pkg| {
            RepoMappingPackageMetadata::new(
                pkg.package_identifier.clone(),
                pkg.repository_mapping.clone(),
            )
        }));
        for (_, package_set) in &self.configured_target_packages {
            result.add_transitive(package_set);
        }
        // PORT: aspectPackages are empty (AspectFunction is not ported).
        result.build()
    }
}

impl TransitiveDependencyState {
    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:78 TransitiveDependencyState.<init>
    pub fn new(store_transitive_packages: bool) -> Self {
        TransitiveDependencyState {
            transitive_root_causes: NestedSetBuilder::stable_order(),
            package_collector: if store_transitive_packages {
                Some(PackageCollector {
                    packages: Vec::new(),
                    configured_target_packages: Vec::new(),
                })
            } else {
                None
            },
        }
    }

    pub fn transitive_root_causes(&self) -> &NestedSetBuilder<Cause> {
        &self.transitive_root_causes
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:96 TransitiveDependencyState.transitivePackages
    /// PORT: Java's `NestedSet<Package.Metadata>`; Blaze keeps the two fields the runfiles repo
    /// mapping manifest reads.
    pub fn transitive_packages(&self) -> Option<NestedSet<RepoMappingPackageMetadata>> {
        Some(self.package_collector.as_ref()?.build_set())
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:103 TransitiveDependencyState.addTransitiveCauses
    pub fn add_transitive_causes(&mut self, transitive_causes: &NestedSet<Cause>) {
        self.transitive_root_causes
            .add_transitive(transitive_causes);
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:107 TransitiveDependencyState.addTransitiveCause
    pub fn add_transitive_cause(&mut self, cause: Cause) {
        self.transitive_root_causes.add(cause);
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:111 TransitiveDependencyState.hasRootCause
    pub fn has_root_cause(&self) -> bool {
        !self.transitive_root_causes.is_empty()
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:115 TransitiveDependencyState.storeTransitivePackages
    pub fn store_transitive_packages(&self) -> bool {
        self.package_collector.is_some()
    }

    /// Adds to the set of transitive package metadata if `store_transitive_packages` is true.
    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:120 TransitiveDependencyState.updateTransitivePackages
    pub fn update_transitive_packages(&mut self, pkg: &Arc<PackageMetadata>) {
        if let Some(c) = &mut self.package_collector {
            c.packages.push(pkg.clone());
        }
    }

    /// Adds to the set of transitive package metadata if `store_transitive_packages` is true.
    // Java: com/google/devtools/build/lib/analysis/TransitiveDependencyState.java:128 TransitiveDependencyState.updateTransitivePackages
    pub fn update_transitive_packages_of_configured_target(
        &mut self,
        key: &ConfiguredTargetKey,
        packages: &NestedSet<RepoMappingPackageMetadata>,
    ) {
        if let Some(c) = &mut self.package_collector {
            // TreeMap.put: replaces an equal key's value.
            match c
                .configured_target_packages
                .binary_search_by(|(k, _)| configured_target_key_ordering(k, key))
            {
                Ok(i) => c.configured_target_packages[i].1 = packages.clone(),
                Err(i) => c
                    .configured_target_packages
                    .insert(i, (key.clone(), packages.clone())),
            }
        }
    }
}

/// Tagged union of the errors of the dependency map producers (Java `DependencyError`).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyError.java:37 DependencyError
#[derive(Clone, Debug)]
pub enum DependencyError {
    DependencyOptionsParsing(String),
    DependencyTransition(TransitionException),
    Materializer(String),
    /// `InvalidVisibilityDependencyException(label)`.
    InvalidVisibility(Label),
    AspectEvaluation(DependencyEvaluationException),
    AspectCreation(String),
    PlatformMapping(String),
    InvalidPlatform(String),
    TransitionCreation(TransitionCreationException),
    BuildOptionsScope(String),
}

impl DependencyError {
    /// The `Kind` ordinal (declaration order of Java's enum).
    fn kind_ordinal(&self) -> u8 {
        match self {
            DependencyError::DependencyOptionsParsing(_) => 0,
            DependencyError::DependencyTransition(_) => 1,
            DependencyError::Materializer(_) => 2,
            DependencyError::InvalidVisibility(_) => 3,
            DependencyError::AspectEvaluation(_) => 4,
            DependencyError::AspectCreation(_) => 5,
            DependencyError::PlatformMapping(_) => 6,
            DependencyError::InvalidPlatform(_) => 7,
            DependencyError::TransitionCreation(_) => 8,
            DependencyError::BuildOptionsScope(_) => 9,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyError.java:75 DependencyError.isSecondErrorMoreImportant
    pub fn is_second_error_more_important(
        first: &DependencyError,
        second: &DependencyError,
    ) -> bool {
        // There isn't a good way to prioritize when the type matches, so we just keep the first.
        first.kind_ordinal() > second.kind_ordinal()
    }
}

/// Tagged union of the errors of the dependency context producers (Java
/// `DependencyContextError`).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextError.java:27 DependencyContextError
#[derive(Clone, Debug)]
pub enum DependencyContextError {
    /// `ToolchainException`: P's error type, carried with its message and the
    /// `asConfiguredValueCreationException` it converts to.
    Toolchain(blaze_analysis::toolchains::ToolchainException),
    ConfiguredValueCreation(ConfiguredValueCreationException),
    /// `IncompatibleTargetException`: the incompatible target's value.
    IncompatibleTarget(Arc<blaze_analysis::configured_target_value::ConfiguredTargetValue>),
    /// `ConfiguredAttributeMapper.ValidationException`.
    Validation(blaze_analysis::config::configured_attribute_mapper::ValidationException),
}

/// A dependency edge whose target could not be loaded (Java `MissingEdgeError`).
// Java: com/google/devtools/build/lib/analysis/producers/MissingEdgeError.java:36 MissingEdgeError
#[derive(Clone, Debug)]
pub struct MissingEdgeError {
    kind: DependencyKind,
    label: Label,
    cause: NoSuchThingException,
}

impl MissingEdgeError {
    pub fn new(kind: DependencyKind, label: Label, cause: NoSuchThingException) -> Self {
        MissingEdgeError { kind, label, cause }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/MissingEdgeError.java:48 MissingEdgeError.emitCausesAndEvents
    pub fn emit_causes_and_events(
        &self,
        from_node: &TargetAndConfiguration,
        transitive_state: &mut TransitiveDependencyState,
        listener: &dyn ExtendedEventHandler,
    ) {
        let from = from_node.get_target();
        let label = &self.label;
        let cause = &self.cause;

        if let NoSuchThingException::Package(p) = cause {
            if p.kind() == NoSuchPackageKind::RepositoryFetch {
                let repository_label = Label::create_in_package(
                    &blaze_cmdline::label_constants::EXTERNAL_PACKAGE_IDENTIFIER,
                    label.get_repository().get_name(),
                )
                .unwrap_or_else(|_| label.clone());
                transitive_state.add_transitive_cause(Cause::LoadingFailed {
                    label: repository_label,
                    detailed_exit_code: cause.get_detailed_exit_code(),
                });
                listener.handle(&Event::error_at(
                    event_location(get_location_maybe(from).as_ref()),
                    format!(
                        "{} depends on {} in repository {} which failed to fetch. {}",
                        from.get_label(),
                        label,
                        label.get_repository(),
                        cause.get_message()
                    ),
                ));
                return;
            }
        }

        match cause {
            NoSuchThingException::Package(_) => {
                // Blames the rule for specifying an unavailable package.
                let configuration = from_node.get_configuration();
                // AnalysisRootCauseEvent.withConfigurationValue(configuration, label, message)
                listener.post(Arc::new(
                    blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent {
                        configuration_checksum: configuration.map(|c| c.checksum().to_owned()),
                        label: label.clone(),
                        error_message: cause.get_message().to_owned(),
                    },
                ));
                transitive_state.add_transitive_cause(Cause::AnalysisFailed {
                    label: label.clone(),
                    configuration: configuration_id_message(configuration.map(|c| c.checksum())),
                    detailed_exit_code: cause.get_detailed_exit_code(),
                });
            }
            NoSuchThingException::Target(t) => {
                // If the child target was present, it already has an associated
                // LoadingFailedCause.
                if !t.has_target() {
                    transitive_state.add_transitive_cause(Cause::LoadingFailed {
                        label: label.clone(),
                        detailed_exit_code: cause.get_detailed_exit_code(),
                    });
                }
            }
            _ => {}
        }

        let message = if self.kind.is_toolchain() {
            format!(
                "Target '{}' depends on toolchain '{}', which cannot be found: {}'",
                from.get_label(),
                label,
                cause.get_message()
            )
        } else {
            format_missing_edge_with_attribute(
                Some(from),
                label,
                cause,
                self.kind.get_attribute().map(|a| &**a),
            )
        };
        listener.handle(&Event::error_at(
            event_location(get_location_maybe(from).as_ref()),
            message,
        ));
    }
}

impl fmt::Display for MissingEdgeError {
    // Java: com/google/devtools/build/lib/analysis/producers/MissingEdgeError.java:113 MissingEdgeError.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MissingEdgeError{{kind={:?}, label={}, cause={}}}",
            self.kind, self.label, self.cause
        )
    }
}
