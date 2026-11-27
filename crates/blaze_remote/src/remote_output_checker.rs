//! Port of `lib/remote/RemoteOutputChecker.java` (with `ConcurrentArtifactPathTrie.java`, which
//! only this class uses).
//!
//! PORT: the hooks that Java calls with analysis types (`ConfiguredTarget`, `ConfiguredAspect`,
//! `TopLevelArtifactContext`, `AnalysisResult`, `MemoizingEvaluator`) take what the method reads
//! of them instead, through the small views declared here ([`TopLevelTarget`],
//! [`TargetUnderTest`], [`AnalysisResultView`]): those types live in crates above this one. The
//! caller (the remote module's wiring) implements the views; each view method names the Java
//! expression it stands for.

use std::collections::{BTreeSet, HashSet};
use std::ops::Bound;
use std::sync::Arc;

use blaze_events::Clock;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, FileArtifactValue, HierarchicalPath, OutputChecker,
};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_vfs::PathFragment;
use parking_lot::{Mutex, RwLock};

/// A pattern of `--remote_download_regex` (Java: `Predicate<String>`,
/// `RegexPatternOption.matcher()`): matches the entire input string against the regex.
pub type PatternToDownload = Arc<dyn Fn(&str) -> bool + Send + Sync>;

// Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:55 RemoteOutputChecker.CommandMode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandMode {
    Unknown,
    Build,
    Test,
    Run,
    Coverage,
}

/// A specialized concurrent trie that stores paths of artifacts and allows checking whether a
/// given path is contained in (in the case of a tree artifact) or exactly matches (in any other
/// case) an artifact in the trie.
// Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:27 ConcurrentArtifactPathTrie
#[derive(Debug, Default)]
struct ConcurrentArtifactPathTrie {
    // Invariant: no path in this set is a prefix of another path.
    // Java: `ConcurrentSkipListSet<PathFragment>` with `PathFragment.HIERARCHICAL_COMPARATOR`.
    paths: RwLock<BTreeSet<HierarchicalPath>>,
}

impl ConcurrentArtifactPathTrie {
    /// Adds the given `ActionInput` to the trie.
    ///
    /// The caller must ensure that no object's path passed to this method is a prefix of any
    /// previously added object's path. Bazel enforces this for non-aggregate artifacts. Callers
    /// must not pass in `Artifact.TreeFileArtifact`s (which have exec paths that have their
    /// parent tree artifact's exec path as a prefix) or non-Artifact `ActionInput`s that violate
    /// this invariant.
    // Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:41 ConcurrentArtifactPathTrie.add
    fn add(&self, input: &dyn ActionInput) {
        blaze_util::check_argument!(
            !is_tree_file_artifact(input),
            "TreeFileArtifacts should not be added to the trie: {:?}",
            input
        );
        self.paths
            .write()
            .insert(HierarchicalPath(input.get_exec_path().clone()));
    }

    /// Checks whether the given `PathFragment` is contained in an artifact in the trie.
    // Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:50 ConcurrentArtifactPathTrie.contains
    fn contains(&self, exec_path: &PathFragment) -> bool {
        // By the invariant of this set, there is at most one prefix of execPath in the set. Since
        // the comparator sorts all children of a path right after the path itself, if such a
        // prefix exists, it must thus sort right before execPath (or be equal to it).
        let paths = self.paths.read();
        // `paths.floor(execPath)`: the greatest element less than or equal to `execPath`.
        let floor_path = paths
            .range((
                Bound::Unbounded,
                Bound::Included(HierarchicalPath(exec_path.clone())),
            ))
            .next_back();
        match floor_path {
            Some(floor_path) => exec_path.starts_with(&floor_path.0),
            None => false,
        }
    }
}

/// `input instanceof Artifact.TreeFileArtifact`.
fn is_tree_file_artifact(input: &dyn ActionInput) -> bool {
    input
        .as_artifact()
        .is_some_and(|artifact| artifact.is_child_of_declared_directory())
}

/// `metadata.isInMemoryOutput()`.
///
/// PORT: `blaze_exec`'s seam copy of `FileArtifactValue` has the field but not the method.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:156 FileArtifactValue.isInMemoryOutput
fn is_in_memory_output(metadata: &FileArtifactValue) -> bool {
    match metadata {
        // Java: FileArtifactValue.java:898 RemoteFileArtifactValueWithMaterializationData.isInMemoryOutput
        FileArtifactValue::RemoteFile {
            in_memory_output, ..
        } => in_memory_output.0,
        // Java: FileArtifactValue.java:1002 ResolvedSymlinkArtifactValue.isInMemoryOutput
        FileArtifactValue::ResolvedSymlink { delegate, .. } => is_in_memory_output(delegate),
        // Java: FileArtifactValue.java:1362 ProxyFileArtifactValue.isInMemoryOutput
        FileArtifactValue::Proxy { delegate, .. } => is_in_memory_output(delegate),
        _ => false,
    }
}

/// `metadata.getExpirationTime()`, as milliseconds since the epoch (`None` is Java's null).
///
/// PORT: `blaze_exec`'s seam copy of `FileArtifactValue` has the field but not the method.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:210 FileArtifactValue.getExpirationTime
fn get_expiration_time(metadata: &FileArtifactValue) -> Option<i64> {
    match metadata {
        // Java: FileArtifactValue.java:857 RemoteFileArtifactValueWithMaterializationData.getExpirationTime
        // (`fromEpochMilli`: a negative value stands for null).
        FileArtifactValue::RemoteFile {
            expiration_time_millis,
            ..
        } => expiration_time_millis
            .0
            .get()
            .filter(|expiration_time| *expiration_time >= 0),
        // Java: FileArtifactValue.java:1053 ResolvedSymlinkArtifactValue.getExpirationTime
        FileArtifactValue::ResolvedSymlink { delegate, .. } => get_expiration_time(delegate),
        _ => None,
    }
}

/// The runfiles of a top-level target, as far as `RemoteOutputChecker.addRunfiles` reads them.
#[derive(Debug, Clone, Default)]
pub struct RunfilesView {
    /// `runfiles.getArtifacts().toList()`.
    pub artifacts: Vec<ActionInputRef>,
    /// `symlink.getArtifact()` of `runfiles.getSymlinks().toList()`.
    pub symlink_artifacts: Vec<ActionInputRef>,
    /// `symlink.getArtifact()` of `runfiles.getRootSymlinks().toList()`.
    pub root_symlink_artifacts: Vec<ActionInputRef>,
}

/// What `RemoteOutputChecker.addTopLevelTarget` reads of a `ProviderCollection` (a configured
/// target or a configured aspect) and of the `TopLevelArtifactContext` its supplier gives.
///
/// PORT: see the module comment. The methods are called in Java's order and only when Java
/// evaluates the expression (in particular, nothing is called when the target is not added, as
/// Java does not call the supplier then).
pub trait TopLevelTarget {
    /// `TopLevelArtifactHelper.getAllArtifactsToBuild(target,
    /// topLevelArtifactContextSupplier.get()).getImportantArtifacts().toList()`.
    fn get_important_artifacts_to_build(&self) -> Vec<ActionInputRef>;

    /// `topLevelArtifactContext.outputGroups().contains(OutputGroupInfo.HIDDEN_TOP_LEVEL)`.
    fn output_groups_contain_hidden_top_level(&self) -> bool;

    /// `target.getProvider(FilesToRunProvider.class).getRunfilesSupport().getRunfiles()`, or
    /// `None` when the provider or its runfiles support is null.
    fn get_runfiles(&self) -> Option<RunfilesView>;

    /// `target.getProvider(ExtraActionArtifactsProvider.class).getExtraActionArtifacts()
    /// .toList()`, or `None` when the target has no such provider.
    fn get_extra_action_artifacts(&self) -> Option<Vec<ActionInputRef>>;
}

/// What `RemoteOutputChecker.addTargetUnderTest` reads of a test target's `TestProvider`
/// (`checkNotNull(target.getProvider(TestProvider.class))` is the caller's: a target without
/// the provider cannot be given as this view).
pub trait TargetUnderTest {
    /// `testProvider.getTestParams().getOutputs()`.
    fn get_test_outputs(&self) -> Vec<ActionInputRef>;

    /// `testProvider.getTestParams().getCoverageArtifacts()`.
    fn get_coverage_artifacts(&self) -> Vec<ActionInputRef>;
}

/// What `RemoteOutputChecker.afterAnalysis` reads of an `AnalysisResult`.
pub struct AnalysisResultView<'a> {
    /// `analysisResult.getTargetsToBuild()`: for each target, its rule class string
    /// (`configuredTarget instanceof RuleConfiguredTarget r ? r.getRuleClassString() : null`)
    /// and its view with `analysisResult::getTopLevelContext` as the context supplier.
    pub targets_to_build: Vec<(Option<&'a str>, &'a dyn TopLevelTarget)>,
    /// `analysisResult.getAspectsMap().values()`, with the same context supplier.
    pub aspects: Vec<&'a dyn TopLevelTarget>,
    /// `analysisResult.getTargetsToTest()` (nullable).
    pub targets_to_test: Option<Vec<&'a dyn TargetUnderTest>>,
    /// `analysisResult.getArtifactsToBuild()`.
    pub artifacts_to_build: Vec<ActionInputRef>,
}

/// An `OutputChecker` that decides which outputs to download taking into account the output mode
/// and the TTL of remote metadata.
// Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:54 RemoteOutputChecker
pub struct RemoteOutputChecker {
    command_mode: CommandMode,
    outputs_mode: RemoteOutputsMode,
    last_remote_output_checker: Option<Arc<RemoteOutputChecker>>,

    // Java: `@Nullable private Clock clock`, set after construction by `setCheckMetadataTtl`.
    clock: RwLock<Option<Arc<dyn Clock>>>,

    patterns_to_download: Vec<PatternToDownload>,
    paths_to_download: ConcurrentArtifactPathTrie,
    // Java: `ConcurrentHashMap.newKeySet()`.
    paths_to_skip: Mutex<HashSet<PathFragment>>,
}

impl std::fmt::Debug for RemoteOutputChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteOutputChecker")
            .field("command_mode", &self.command_mode)
            .field("outputs_mode", &self.outputs_mode)
            .field(
                "last_remote_output_checker",
                &self.last_remote_output_checker,
            )
            .field("patterns_to_download", &self.patterns_to_download.len())
            .field("paths_to_download", &self.paths_to_download)
            .field("paths_to_skip", &self.paths_to_skip)
            .finish_non_exhaustive()
    }
}

impl RemoteOutputChecker {
    /// `new RemoteOutputChecker(commandName, outputsMode, patternsToDownload)`.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:73 RemoteOutputChecker.<init>
    pub fn new(
        command_name: &str,
        outputs_mode: RemoteOutputsMode,
        patterns_to_download: Vec<PatternToDownload>,
    ) -> RemoteOutputChecker {
        RemoteOutputChecker::new_with_last_remote_output_checker(
            command_name,
            outputs_mode,
            patterns_to_download,
            /* lastRemoteOutputChecker= */ None,
        )
    }

    /// `new RemoteOutputChecker(commandName, outputsMode, patternsToDownload,
    /// lastRemoteOutputChecker)`.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:80 RemoteOutputChecker.<init>
    pub fn new_with_last_remote_output_checker(
        command_name: &str,
        outputs_mode: RemoteOutputsMode,
        patterns_to_download: Vec<PatternToDownload>,
        last_remote_output_checker: Option<Arc<RemoteOutputChecker>>,
    ) -> RemoteOutputChecker {
        let command_mode = match command_name {
            "build" => CommandMode::Build,
            "test" => CommandMode::Test,
            "run" => CommandMode::Run,
            "coverage" => CommandMode::Coverage,
            _ => CommandMode::Unknown,
        };
        RemoteOutputChecker {
            command_mode,
            outputs_mode,
            last_remote_output_checker,
            clock: RwLock::new(None),
            patterns_to_download,
            paths_to_download: ConcurrentArtifactPathTrie::default(),
            paths_to_skip: Mutex::new(HashSet::new()),
        }
    }

    /// The `patternsToDownload` that `RemoteModule.beforeCommand` builds from
    /// `--remote_download_regex` for the constructor.
    ///
    /// PORT: this is the part of `RemoteModule` that produces the constructor's argument; it is
    /// here so that the predicates are built in one place from the option type.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:561 RemoteModule.beforeCommand
    pub fn patterns_to_download(remote_options: &RemoteOptions) -> Vec<PatternToDownload> {
        // TODO(bazel-team): Consider adding a warning or more validation if the
        // remoteDownloadRegex is used without Build without the Bytes.
        let mut patterns_to_download_builder: Vec<PatternToDownload> = Vec::new();
        if remote_options.remote_outputs_mode != RemoteOutputsMode::ALL {
            for pattern_option in &remote_options.remote_download_regex {
                // `patternOption.matcher()`: matches the entire input string.
                let pattern_option = pattern_option.clone();
                patterns_to_download_builder
                    .push(Arc::new(move |s: &str| pattern_option.matches(s)));
            }
        }
        patterns_to_download_builder
    }

    /// Sets this checker to check the TTL of remote metadata when deciding whether to trust it.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:99 RemoteOutputChecker.setCheckMetadataTtl
    pub fn set_check_metadata_ttl(&self, clock: Arc<dyn Clock>) {
        *self.clock.write() = Some(clock);
    }

    /// Skymeld-only.
    ///
    /// PORT: `configured_target_rule_class_string` is `configuredTarget instanceof
    /// RuleConfiguredTarget r ? r.getRuleClassString() : null`; `target` is the view of
    /// `configuredTarget` with `topLevelArtifactContextSupplier`.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:104 RemoteOutputChecker.afterTopLevelTargetAnalysis
    pub fn after_top_level_target_analysis(
        &self,
        configured_target_rule_class_string: Option<&str>,
        target: &dyn TopLevelTarget,
    ) {
        if self.outputs_mode == RemoteOutputsMode::ALL {
            // For ALL, there's no need to keep track of toplevel targets - we download
            // everything.
            return;
        }
        self.add_top_level_target(target, configured_target_rule_class_string);
    }

    /// Skymeld-only.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:115 RemoteOutputChecker.afterTestAnalyzedEvent
    pub fn after_test_analyzed_event(&self, configured_target: &dyn TargetUnderTest) {
        if self.outputs_mode == RemoteOutputsMode::ALL {
            // For ALL, there's no need to keep track of toplevel targets - we download
            // everything.
            return;
        }
        self.add_target_under_test(configured_target);
    }

    /// Skymeld-only.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:124 RemoteOutputChecker.afterAspectAnalysis
    pub fn after_aspect_analysis(&self, configured_aspect: &dyn TopLevelTarget) {
        if self.outputs_mode == RemoteOutputsMode::ALL {
            // For ALL, there's no need to keep track of toplevel targets - we download
            // everything.
            return;
        }
        self.add_top_level_target(configured_aspect, /* configuredTarget= */ None);
    }

    /// Skymeld-only.
    ///
    /// PORT: `is_owned_by_coverage_report` is `artifact.getArtifactOwner().equals(
    /// CoverageReportValue.COVERAGE_REPORT_KEY)` (the key lives in a crate above this one).
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:136 RemoteOutputChecker.coverageArtifactsKnown
    pub fn coverage_artifacts_known(
        &self,
        coverage_artifacts: &[ActionInputRef],
        is_owned_by_coverage_report: &dyn Fn(&ActionInputRef) -> bool,
    ) {
        if self.outputs_mode == RemoteOutputsMode::ALL {
            // For ALL, there's no need to keep track of toplevel targets - we download
            // everything.
            return;
        }
        self.maybe_add_coverage_artifacts(coverage_artifacts, is_owned_by_coverage_report);
    }

    /// Non-Skymeld only.
    ///
    /// PORT: `is_owned_by_coverage_report` as in [`RemoteOutputChecker::coverage_artifacts_known`].
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:145 RemoteOutputChecker.afterAnalysis
    pub fn after_analysis(
        &self,
        analysis_result: &AnalysisResultView<'_>,
        is_owned_by_coverage_report: &dyn Fn(&ActionInputRef) -> bool,
    ) {
        if self.outputs_mode == RemoteOutputsMode::ALL {
            // For ALL, there's no need to keep track of toplevel targets - we download
            // everything.
            return;
        }
        for (rule_class_string, target) in &analysis_result.targets_to_build {
            self.add_top_level_target(*target, *rule_class_string);
        }
        for aspect in &analysis_result.aspects {
            self.add_top_level_target(*aspect, /* configuredTarget= */ None);
        }
        let targets_to_test = &analysis_result.targets_to_test;
        if let Some(targets_to_test) = targets_to_test {
            for target in targets_to_test {
                self.add_target_under_test(*target);
            }
            self.maybe_add_coverage_artifacts(
                &analysis_result.artifacts_to_build,
                is_owned_by_coverage_report,
            );
        }
    }

    /// PORT: Java's `@Nullable ConfiguredTarget configuredTarget` is only read by
    /// `shouldAddTopLevelTarget`, which reads its rule class string when it is a
    /// `RuleConfiguredTarget`: `configured_target_rule_class_string` is that string, `None` when
    /// `configuredTarget` is null or not a `RuleConfiguredTarget`.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:165 RemoteOutputChecker.addTopLevelTarget
    fn add_top_level_target(
        &self,
        target: &dyn TopLevelTarget,
        configured_target_rule_class_string: Option<&str>,
    ) {
        if self.should_add_top_level_target(configured_target_rule_class_string) {
            let artifacts_to_build = target.get_important_artifacts_to_build();
            self.add_outputs_to_download(&artifacts_to_build);
            // RunfileTrees are requested with this special output group. We lack access to an
            // InputMetadataProvider that can expand arbitrary RunfileTrees, so we have to mirror
            // that logic here.
            if target.output_groups_contain_hidden_top_level() {
                self.add_runfiles(target);
            }
            self.add_extra_action_artifacts(target);
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:185 RemoteOutputChecker.addRunfiles
    fn add_runfiles(&self, build_target: &dyn TopLevelTarget) {
        // `runfilesProvider == null` and `runfilesSupport == null` are the `None` of the view.
        let Some(runfiles) = build_target.get_runfiles() else {
            return;
        };
        for runfile in &runfiles.artifacts {
            if RemoteOutputChecker::may_be_remote(&**runfile) {
                self.add_output_to_download(&**runfile);
            }
        }
        for artifact in &runfiles.symlink_artifacts {
            if RemoteOutputChecker::may_be_remote(&**artifact) {
                self.add_output_to_download(&**artifact);
            }
        }
        for artifact in &runfiles.root_symlink_artifacts {
            if RemoteOutputChecker::may_be_remote(&**artifact) {
                self.add_output_to_download(&**artifact);
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:214 RemoteOutputChecker.addExtraActionArtifacts
    fn add_extra_action_artifacts(&self, target: &dyn TopLevelTarget) {
        let extra_action_artifacts = target.get_extra_action_artifacts();
        if let Some(extra_action_artifacts) = extra_action_artifacts {
            self.add_outputs_to_download(&extra_action_artifacts);
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:222 RemoteOutputChecker.addTargetUnderTest
    fn add_target_under_test(&self, target: &dyn TargetUnderTest) {
        if self.outputs_mode != RemoteOutputsMode::MINIMAL
            && (self.command_mode == CommandMode::Test
                || self.command_mode == CommandMode::Coverage)
        {
            // In test or coverage mode, download the outputs of the test runner action.
            self.add_outputs_to_download(&target.get_test_outputs());
        }
        if self.command_mode == CommandMode::Coverage {
            // In coverage mode, download the per-test and aggregated coverage files.
            // Do this even for MINIMAL, since coverage (unlike test) doesn't produce any
            // observable results other than outputs.
            self.add_outputs_to_download(&target.get_coverage_artifacts());
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:237 RemoteOutputChecker.maybeAddCoverageArtifacts
    fn maybe_add_coverage_artifacts(
        &self,
        artifacts_to_build: &[ActionInputRef],
        is_owned_by_coverage_report: &dyn Fn(&ActionInputRef) -> bool,
    ) {
        if self.command_mode != CommandMode::Coverage {
            return;
        }
        for artifact_to_build in artifacts_to_build {
            if is_owned_by_coverage_report(artifact_to_build) {
                self.add_output_to_download(&**artifact_to_build);
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:248 RemoteOutputChecker.addOutputsToDownload
    fn add_outputs_to_download(&self, files: &[ActionInputRef]) {
        for file in files {
            self.add_output_to_download(&**file);
        }
    }

    /// Marks a file for download.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:255 RemoteOutputChecker.addOutputToDownload
    pub fn add_output_to_download(&self, file: &dyn ActionInput) {
        self.paths_to_download.add(file);
    }

    /// Marks a file as not for download, regardless of the output mode.
    ///
    /// This is used by `RemoteExecutionService` to skip downloading in-memory outputs.
    ///
    /// `exec_path`: the exec path of the file that is not to be downloaded.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:266 RemoteOutputChecker.skipDownload
    pub fn skip_download(&self, exec_path: &PathFragment) {
        self.paths_to_skip.lock().insert(exec_path.clone());
    }

    /// PORT: `configured_target_rule_class_string` is `Some` when Java's `configuredTarget` is a
    /// `RuleConfiguredTarget` (its `getRuleClassString()`), `None` when it is null or another
    /// kind of configured target.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:270 RemoteOutputChecker.shouldAddTopLevelTarget
    fn should_add_top_level_target(
        &self,
        configured_target_rule_class_string: Option<&str>,
    ) -> bool {
        match self.command_mode {
            // Always download outputs of toplevel targets in run mode.
            CommandMode::Run => true,
            CommandMode::Coverage | CommandMode::Test => {
                // Do not download test binary in test/coverage mode.
                if let Some(rule_class_string) = configured_target_rule_class_string
                    && is_test_rule_name(rule_class_string)
                {
                    return false;
                }
                self.outputs_mode != RemoteOutputsMode::MINIMAL
            }
            _ => self.outputs_mode != RemoteOutputsMode::MINIMAL,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:286 RemoteOutputChecker.matchesPattern
    fn matches_pattern(&self, exec_path: &PathFragment) -> bool {
        for pattern in &self.patterns_to_download {
            // `execPath.toString()`
            if pattern(exec_path.get_path_string()) {
                return true;
            }
        }
        false
    }

    /// Returns whether this `ActionInput` could conceivably be only available remotely.
    ///
    /// Use this as a quick check to avoid unnecessary extra work for artifacts that are
    /// definitely local.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:301 RemoteOutputChecker.mayBeRemote
    pub fn may_be_remote(action_input: &dyn ActionInput) -> bool {
        !(action_input.as_artifact().is_some_and(|artifact| {
            artifact.is_source_artifact()
                // Source artifacts in the main repo don't need to be fetched.
                && source_artifact_owner_is_null_or_in_main_repository(action_input)
        }))
    }

    /// Returns whether a remote `ActionInput` with the given path should be downloaded.
    ///
    /// `tree_root_exec_path`: the path of the tree artifact if the given `ActionInput` is
    /// contained in one.
    ///
    /// PORT: Java overloads `shouldDownloadOutput`; this is the `(PathFragment, PathFragment)`
    /// overload. The `(ActionInput, FileArtifactValue)` overload is
    /// [`OutputChecker::should_download_output`].
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:328 RemoteOutputChecker.shouldDownloadOutput
    pub fn should_download_output_path(
        &self,
        exec_path: &PathFragment,
        tree_root_exec_path: Option<&PathFragment>,
    ) -> bool {
        if self.paths_to_skip.lock().contains(exec_path) {
            return false;
        }
        self.outputs_mode == RemoteOutputsMode::ALL
            || self.paths_to_download.contains(exec_path)
            || self.matches_pattern(exec_path)
            || tree_root_exec_path
                .is_some_and(|tree_root_exec_path| self.matches_pattern(tree_root_exec_path))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:377 RemoteOutputChecker.isAlive
    fn is_alive(&self, metadata: &FileArtifactValue, clock: &dyn Clock) -> bool {
        let expiration_time = get_expiration_time(metadata);
        // `expirationTime == null || expirationTime.isAfter(clock.now())`; `clock.now()` is
        // `Instant.ofEpochMilli(currentTimeMillis())` and the expiration time is kept in
        // milliseconds.
        match expiration_time {
            None => true,
            Some(expiration_time) => expiration_time > clock.current_time_millis(),
        }
    }

    /// `maybeInvalidateSkyframeValues(memoizingEvaluator)`.
    ///
    /// PORT: `memoizing_evaluator_delete` is `memoizingEvaluator.delete(predicate)`; the
    /// predicate it receives takes `k.functionName()` by its name (`SkyFunctions.TARGET_COMPLETION`
    /// and `SkyFunctions.ASPECT_COMPLETION` are "TARGET_COMPLETION" and "ASPECT_COMPLETION").
    /// The evaluator lives in a crate above this one.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:382 RemoteOutputChecker.maybeInvalidateSkyframeValues
    #[allow(clippy::type_complexity)]
    pub fn maybe_invalidate_skyframe_values(
        &self,
        memoizing_evaluator_delete: &mut dyn FnMut(&dyn Fn(&str) -> bool),
    ) {
        let Some(last_remote_output_checker) = &self.last_remote_output_checker else {
            return;
        };

        // Run mode is part of TopLevelArtifactContext, and thus of target and aspect completion
        // keys, so transitions to and from it do not require manual invalidation.
        let run_transition = last_remote_output_checker.command_mode == CommandMode::Run
            || self.command_mode == CommandMode::Run;
        if last_remote_output_checker.outputs_mode != self.outputs_mode
            || (last_remote_output_checker.command_mode != self.command_mode && !run_transition)
        {
            memoizing_evaluator_delete(&|function_name: &str| {
                function_name == "TARGET_COMPLETION" || function_name == "ASPECT_COMPLETION"
            });
        }
    }
}

impl OutputChecker for RemoteOutputChecker {
    /// Returns whether this `ActionInput` should be downloaded.
    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:310 RemoteOutputChecker.shouldDownloadOutput
    fn should_download_output(
        &self,
        output: &dyn ActionInput,
        metadata: &FileArtifactValue,
    ) -> bool {
        blaze_util::check_state!(
            !(output
                .as_artifact()
                .is_some_and(|artifact| artifact.is_tree_artifact())),
            "shouldDownloadOutput should not be called on a tree artifact"
        );
        metadata.is_remote() && {
            // `output instanceof TreeFileArtifact artifact ? artifact.getParent().getExecPath()
            // : null`
            let parent = if is_tree_file_artifact(output) {
                Some(
                    output
                        .as_artifact()
                        .and_then(|artifact| artifact.get_parent())
                        .expect("NullPointerException: TreeFileArtifact.getParent()"),
                )
            } else {
                None
            };
            self.should_download_output_path(
                output.get_exec_path(),
                parent.as_ref().map(|parent| parent.get_exec_path()),
            )
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:340 RemoteOutputChecker.shouldTrustMetadata
    fn should_trust_metadata(&self, file: &dyn ActionInput, metadata: &FileArtifactValue) -> bool {
        // Local metadata is always trusted.
        if !metadata.is_remote() {
            return true;
        }

        // An in-memory output (e.g. --experimental_inmemory_{dotd,jdeps}_files) is never written
        // to disk, so a "should download" verdict must not force a re-execution to materialize
        // it; trust it via its TTL instead. See https://github.com/bazelbuild/bazel/issues/29313.
        if !is_in_memory_output(metadata) {
            // If Bazel should download this file, but it does not exist locally, returns false to
            // rerun the generating action to trigger the download (just like in the normal build,
            // when local outputs are missing).
            if let Some(last_remote_output_checker) = &self.last_remote_output_checker {
                // This is an incremental build. If the file was downloaded by previous build and
                // is now missing, invalidate the action.
                if last_remote_output_checker.should_download_output(file, metadata) {
                    return false;
                }
            }

            if self.should_download_output(file, metadata) {
                return false;
            }
        }

        let clock = self.clock.read().clone();
        if let Some(clock) = clock {
            return self.is_alive(metadata, &*clock);
        }

        // The remote metadata may have passed its TTL, but we are requested to optimistically
        // assume that it's still available remotely. If it isn't, build or action rewinding will
        // take care of rerunning the actions needed to produce the file and also evict the stale
        // metadata. This incurs roughly the same performance hit, but only when actually needed.
        true
    }
}

/// `TargetUtils.isTestRuleName(name)`.
///
/// PORT: the port of `TargetUtils` is in `blaze_packages`, which this crate does not depend on.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:67 TargetUtils.isTestRuleName
fn is_test_rule_name(name: &str) -> bool {
    name.ends_with("_test")
}

/// `artifact.getOwner() == null || artifact.getOwner().getRepository().isMain()` for the source
/// artifact `action_input`.
///
/// PORT: `blaze_exec`'s `Artifact` view has no `getOwner()`; the owner is read from the
/// `blaze_actions` artifact behind the view. A source artifact of another type (a test double)
/// has no owner to read.
// Java: com/google/devtools/build/lib/remote/RemoteOutputChecker.java:301 RemoteOutputChecker.mayBeRemote
fn source_artifact_owner_is_null_or_in_main_repository(action_input: &dyn ActionInput) -> bool {
    match action_input
        .as_any()
        .downcast_ref::<blaze_actions::actions::artifact::Artifact>()
    {
        Some(artifact) => match artifact.get_owner() {
            None => true,
            Some(owner) => owner.get_repository().is_main(),
        },
        None => blaze_util::unported!(
            "RemoteOutputChecker.mayBeRemote (missing hook: Artifact.getOwner() on blaze_exec::actions::Artifact)"
        ),
    }
}
