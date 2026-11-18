//! Port of `buildtool/BuildResult.java`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_protos::failure_details::FailureDetail;
use blaze_sky_exec::analysis_seam::ConfiguredTargetView;
use blaze_util::{check_state, unported};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};

/// A configured target as the build driver sees it (Java `ConfiguredTarget`).
pub type ConfiguredTargetRef = Arc<dyn ConfiguredTargetView>;

/// Java `Collection<ConfiguredTarget>.contains`: configured targets are equal when they are the
/// same object or have the same `ConfiguredTargetKey`.
pub fn contains_target(targets: &[ConfiguredTargetRef], target: &ConfiguredTargetRef) -> bool {
    targets
        .iter()
        .any(|t| Arc::ptr_eq(t, target) || t.get_lookup_key() == target.get_lookup_key())
}

/// The parts of `DetailedExitCode` that the classes of this crate read.
///
/// PORT: `DetailedExitCode` (with `ExitCode` and the `FailureDetails.metadata` exit-code table)
/// is ported in `blaze_runtime`, which sits above this crate. `blaze_runtime` implements this
/// trait for it; [`DetailedExitCodeView::as_any`] gets the concrete value back.
// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:31 DetailedExitCode
pub trait DetailedExitCodeView: Send + Sync + fmt::Debug + fmt::Display {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:58 DetailedExitCode.isSuccess
    fn is_success(&self) -> bool;

    /// `getExitCode().isInfrastructureFailure()`.
    // Java: com/google/devtools/build/lib/util/ExitCode.java:139 ExitCode.isInfrastructureFailure
    fn is_infrastructure_failure(&self) -> bool;

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:54 DetailedExitCode.getFailureDetail
    fn get_failure_detail(&self) -> Option<&FailureDetail>;

    fn as_any(&self) -> &dyn Any;
}

/// A shared [`DetailedExitCodeView`].
pub type DetailedExitCodeRef = Arc<dyn DetailedExitCodeView>;

/// The `DetailedExitCode` constructors the classes of this crate call.
///
/// PORT: see [`DetailedExitCodeView`]; `blaze_runtime` implements this with its
/// `DetailedExitCode` (`of` needs the `FailureDetails.metadata` exit-code table it generates).
pub trait DetailedExitCodeFactory: Send + Sync {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:86 DetailedExitCode.of
    fn of(&self, failure_detail: FailureDetail) -> DetailedExitCodeRef;
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:63 DetailedExitCode.success
    fn success(&self) -> DetailedExitCodeRef;
}

/// `DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie`.
// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:190 DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie
pub fn choose_more_important_with_first_if_tie(
    first: Option<DetailedExitCodeRef>,
    second: Option<DetailedExitCodeRef>,
) -> Option<DetailedExitCodeRef> {
    if get_priority(first.as_deref()) - get_priority(second.as_deref()) >= 0 {
        first
    } else {
        second
    }
}

// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:201 DetailedExitCodeComparator.getPriority
fn get_priority(code: Option<&dyn DetailedExitCodeView>) -> i32 {
    match code {
        None => 0,
        Some(code) => {
            let code_class = if code.is_infrastructure_failure() {
                4
            } else {
                2
            };
            code_class
                + if code.get_failure_detail().is_some() {
                    1
                } else {
                    0
                }
        }
    }
}

/// Contains information about the result of a build. While BuildRequest is immutable, this class
/// is mutable.
///
/// PORT: `AspectKey` is an [`ActionLookupKeyRef`] (`AspectKeyCreator.AspectKey` is not ported);
/// `Collection<ConfiguredTarget>` is a `Vec` of [`ConfiguredTargetRef`].
// Java: com/google/devtools/build/lib/buildtool/BuildResult.java:51 BuildResult
pub struct BuildResult {
    start_time_millis: i64, // milliseconds since UNIX epoch.
    stop_time_millis: i64,

    /// PORT: Java keeps the `Throwable`; here its `toString()`.
    crash: Option<String>,
    catastrophe: bool,
    stop_on_first_failure: bool,
    detailed_exit_code: Option<DetailedExitCodeRef>,

    configuration: Option<Arc<BuildConfigurationValue>>,
    convenience_symlinks: IndexMap<PathFragment, PathFragment>,
    actual_targets: Option<Vec<ConfiguredTargetRef>>,
    test_targets: Option<Vec<ConfiguredTargetRef>>,
    successful_targets: Option<Vec<ConfiguredTargetRef>>,
    skipped_targets: Option<Vec<ConfiguredTargetRef>>,
    successful_aspects: Option<IndexSet<ActionLookupKeyRef>>,

    build_tool_log_collection: BuildToolLogCollection,

    post_build_callback_failure_detail: Option<FailureDetail>,
}

impl BuildResult {
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:72 BuildResult.<init>
    pub fn new(start_time_millis: i64) -> BuildResult {
        BuildResult {
            start_time_millis,
            stop_time_millis: 0,
            crash: None,
            catastrophe: false,
            stop_on_first_failure: false,
            detailed_exit_code: None,
            configuration: None,
            convenience_symlinks: IndexMap::new(),
            actual_targets: None,
            test_targets: None,
            successful_targets: None,
            skipped_targets: None,
            successful_aspects: None,
            build_tool_log_collection: BuildToolLogCollection::new(),
            post_build_callback_failure_detail: None,
        }
    }

    /// Record the time (according to System.currentTimeMillis()) at which the service of this
    /// request was completed.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:80 BuildResult.setStopTime
    pub fn set_stop_time(&mut self, stop_time_millis: i64) {
        self.stop_time_millis = stop_time_millis;
    }

    /// Return the time (according to System.currentTimeMillis()) at which the service of this
    /// request was completed.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:88 BuildResult.getStopTime
    pub fn get_stop_time(&self) -> i64 {
        self.stop_time_millis
    }

    /// Returns the elapsed time in seconds for the service of this request. Not defined for
    /// requests that have not been serviced.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:96 BuildResult.getElapsedSeconds
    pub fn get_elapsed_seconds(&self) -> f64 {
        if self.start_time_millis == 0 || self.stop_time_millis == 0 {
            panic!("IllegalStateException: BuildRequest has not been serviced");
        }
        (self.stop_time_millis - self.start_time_millis) as f64 / 1000.0
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:103 BuildResult.setDetailedExitCode
    pub fn set_detailed_exit_code(&mut self, detailed_exit_code: DetailedExitCodeRef) {
        self.detailed_exit_code = Some(detailed_exit_code);
    }

    /// True iff the build request has been successfully completed.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:108 BuildResult.getSuccess
    pub fn get_success(&self) -> bool {
        self.detailed_exit_code
            .as_ref()
            .is_some_and(|c| c.is_success())
    }

    /// Gets the `DetailedExitCode` containing the `ExitCode` and optional failure detail to
    /// complete the command with.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:116 BuildResult.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCodeRef {
        if let Some(code) = &self.detailed_exit_code {
            return code.clone();
        }
        unported!(
            "CrashFailureDetails.detailedExitCodeForThrowable(new IllegalStateException(\"Unspecified DetailedExitCode\"))"
        )
    }

    /// Sets the RuntimeException / Error that induced a Blaze crash.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:125 BuildResult.setUnhandledThrowable
    pub fn set_unhandled_throwable(&mut self, crash: Option<String>) {
        self.crash = crash;
    }

    /// Sets a "catastrophe": A build failure severe enough to halt a keep_going build.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:132 BuildResult.setCatastrophe
    pub fn set_catastrophe(&mut self) {
        self.catastrophe = true;
    }

    /// Was the build a "catastrophe": A build failure severe enough to halt a keep_going build.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:137 BuildResult.wasCatastrophe
    pub fn was_catastrophe(&self) -> bool {
        self.catastrophe
    }

    /// Whether some targets were skipped because of `setStopOnFirstFailure`.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:142 BuildResult.getStopOnFirstFailure
    pub fn get_stop_on_first_failure(&self) -> bool {
        self.stop_on_first_failure
    }

    /// Indicates that remaining targets should be skipped once a target breaks/fails. This will be
    /// set when --nokeep_going or --notest_keep_going is set.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:150 BuildResult.setStopOnFirstFailure
    pub fn set_stop_on_first_failure(&mut self, stop_on_first_failure: bool) {
        self.stop_on_first_failure = stop_on_first_failure;
    }

    /// Gets the Blaze crash Throwable. `None` if Blaze did not crash.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:155 BuildResult.getUnhandledThrowable
    pub fn get_unhandled_throwable(&self) -> Option<&str> {
        self.crash.as_deref()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:159 BuildResult.setBuildConfiguration
    pub fn set_build_configuration(&mut self, configuration: Arc<BuildConfigurationValue>) {
        self.configuration = Some(configuration);
    }

    /// Returns the build configuration collection used for the build.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:164 BuildResult.getBuildConfiguration
    pub fn get_build_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:168 BuildResult.setConvenienceSymlinks
    pub fn set_convenience_symlinks(
        &mut self,
        convenience_symlinks: IndexMap<PathFragment, PathFragment>,
    ) {
        self.convenience_symlinks = convenience_symlinks;
    }

    /// Returns the convenience symlinks for this build in name -> target format (eg blaze-out ->
    /// /symlink/target).
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:176 BuildResult.getConvenienceSymlinks
    pub fn get_convenience_symlinks(&self) -> &IndexMap<PathFragment, PathFragment> {
        &self.convenience_symlinks
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:181 BuildResult.setActualTargets
    pub fn set_actual_targets(&mut self, actual_targets: Option<Vec<ConfiguredTargetRef>>) {
        self.actual_targets = actual_targets;
    }

    /// Returns the actual set of targets which we attempted to build. This value is set during
    /// the build, after the target patterns have been parsed and resolved. If --keep_going is
    /// specified, this set may exclude targets that could not be found or successfully analyzed.
    /// It may be examined after the build. May be null even after the build, if there were errors
    /// in the loading or analysis phases.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:192 BuildResult.getActualTargets
    pub fn get_actual_targets(&self) -> Option<&[ConfiguredTargetRef]> {
        self.actual_targets.as_deref()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:197 BuildResult.setTestTargets
    pub fn set_test_targets(&mut self, test_targets: Option<Vec<ConfiguredTargetRef>>) {
        self.test_targets = test_targets;
    }

    /// Returns the actual unmodifiable collection of targets which we attempted to test. This
    /// value is set at the end of the build analysis phase, after the test target patterns have
    /// been parsed and resolved. If --keep_going is specified, this collection may exclude
    /// targets that could not be found or successfully analyzed. It may be examined after the
    /// build. May be null even after the build, if there were errors in the loading or analysis
    /// phases or if testing was not requested.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:208 BuildResult.getTestTargets
    pub fn get_test_targets(&self) -> Option<&[ConfiguredTargetRef]> {
        self.test_targets.as_deref()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:213 BuildResult.setSuccessfulTargets
    pub fn set_successful_targets(&mut self, successful_targets: Option<Vec<ConfiguredTargetRef>>) {
        self.successful_targets = successful_targets;
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:218 BuildResult.setSuccessfulAspects
    pub fn set_successful_aspects(
        &mut self,
        successful_aspects: Option<IndexSet<ActionLookupKeyRef>>,
    ) {
        self.successful_aspects = successful_aspects;
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:222 BuildResult.setPostBuildCallbackFailureDetail
    pub fn set_post_build_callback_failure_detail(&mut self, failure_detail: FailureDetail) {
        self.post_build_callback_failure_detail = Some(failure_detail);
    }

    /// Only set if build was successful; if callback is successful as well, returns `None`.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:228 BuildResult.getPostBuildCallBackFailureDetail
    pub fn get_post_build_call_back_failure_detail(&self) -> Option<&FailureDetail> {
        self.post_build_callback_failure_detail.as_ref()
    }

    /// Returns the set of targets that were successfully built. This value is set at the end of
    /// the build, after the target patterns have been parsed and resolved and after attempting to
    /// build the targets. If --keep_going is specified, this set may exclude targets that could
    /// not be found or successfully analyzed, or could not be built. It may be examined after the
    /// build. May be null if the execution phase was not attempted, as may happen if there are
    /// errors in the loading phase, for example.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:240 BuildResult.getSuccessfulTargets
    pub fn get_successful_targets(&self) -> Option<&[ConfiguredTargetRef]> {
        self.successful_targets.as_deref()
    }

    /// Returns the set of aspects that were successfully built. This value is set at the end of
    /// the build, after the target patterns have been parsed and resolved and after attempting to
    /// build the targets. If --keep_going is specified, this set may exclude targets that could
    /// not be found or successfully analyzed, or could not be built. It may be examined after the
    /// build. May be null if the execution phase was not attempted, as may happen if there are
    /// errors in the loading phase, for example.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:252 BuildResult.getSuccessfulAspects
    pub fn get_successful_aspects(&self) -> Option<&IndexSet<ActionLookupKeyRef>> {
        self.successful_aspects.as_ref()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:257 BuildResult.setSkippedTargets
    pub fn set_skipped_targets(&mut self, skipped_targets: Option<Vec<ConfiguredTargetRef>>) {
        self.skipped_targets = skipped_targets;
    }

    /// Returns the set of targets which were skipped (Blaze didn't attempt to execute them)
    /// because they're not compatible with the build's target platform.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:265 BuildResult.getSkippedTargets
    pub fn get_skipped_targets(&self) -> Option<&[ConfiguredTargetRef]> {
        self.skipped_targets.as_deref()
    }

    /// Collection of data for the build tool logs event. This may only be modified until the
    /// BuildCompleteEvent is posted; any changes after that event is handled will not be
    /// included in the build tool logs event.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:274 BuildResult.getBuildToolLogCollection
    pub fn get_build_tool_log_collection(&mut self) -> &mut BuildToolLogCollection {
        &mut self.build_tool_log_collection
    }
}

impl fmt::Debug for BuildResult {
    /// For debugging.
    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:280 BuildResult.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildResult")
            .field("startTimeMillis", &self.start_time_millis)
            .field("stopTimeMillis", &self.stop_time_millis)
            .field("crash", &self.crash)
            .field("catastrophe", &self.catastrophe)
            .field("detailedExitCode", &self.detailed_exit_code)
            .field("actualTargets", &self.actual_targets)
            .field("testTargets", &self.test_targets)
            .field("successfulTargets", &self.successful_targets)
            .field("buildToolLogCollection", &self.build_tool_log_collection)
            .finish()
    }
}

/// `BuildEvent.LocalFile.LocalFileType`.
///
/// PORT: Java declares it in `buildeventstream/BuildEvent.java` (BEP, not ported yet). Move it
/// with the BEP port.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:53 BuildEvent.LocalFile.LocalFileType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum LocalFileType {
    OUTPUT_FILE,
    OUTPUT_DIRECTORY,
    OUTPUT_SYMLINK,
    SUCCESSFUL_TEST_OUTPUT,
    FAILED_TEST_OUTPUT,
    COVERAGE_OUTPUT,
    QUERY_OUTPUT,
    STDOUT,
    STDERR,
    LOG,
    PERFORMANCE_LOG,
}

impl LocalFileType {
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:67 LocalFileType.isOutput
    pub fn is_output(self) -> bool {
        self == LocalFileType::OUTPUT_FILE
            || self == LocalFileType::OUTPUT_DIRECTORY
            || self == LocalFileType::OUTPUT_SYMLINK
    }
}

/// `BuildEvent.LocalFile.LocalFileCompression`.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:99 BuildEvent.LocalFile.LocalFileCompression
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum LocalFileCompression {
    NONE,
    GZIP,
}

/// `BuildEvent.LocalFile`: a local file that is referenced by the build event.
///
/// PORT: `artifactMetadata` is not kept (BuildToolLogCollection always passes null).
// Java: com/google/devtools/build/lib/buildeventstream/BuildEvent.java:47 BuildEvent.LocalFile
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub path: Path,
    pub type_: LocalFileType,
    pub compression: LocalFileCompression,
}

/// `BuildToolLogs.LogFileEntry`.
// Java: com/google/devtools/build/lib/buildeventstream/BuildToolLogs.java:118 BuildToolLogs.LogFileEntry
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFileEntry {
    pub name: String,
    pub local_file: LocalFile,
}

/// Collection of data for the build tool logs event. See `BuildToolLogs` for details.
///
/// PORT: Java's `ListenableFuture<String>` URIs are kept as resolved strings (every Bazel caller
/// passes an immediate future or one the BEP transport resolves; neither is ported).
// Java: com/google/devtools/build/lib/buildtool/BuildResult.java:295 BuildResult.BuildToolLogCollection
#[derive(Debug, Default)]
pub struct BuildToolLogCollection {
    direct_values: Vec<(String, Vec<u8>)>,
    future_uris: Vec<(String, String)>,
    local_files: Vec<LogFileEntry>,
    frozen: bool,
}

impl BuildToolLogCollection {
    pub fn new() -> BuildToolLogCollection {
        BuildToolLogCollection::default()
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:302 BuildToolLogCollection.freeze
    pub fn freeze(&mut self) -> &mut Self {
        self.frozen = true;
        self
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:308 BuildToolLogCollection.getLocalFiles
    pub fn get_local_files(&self) -> &[LogFileEntry] {
        &self.local_files
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:313 BuildToolLogCollection.addDirectValue
    pub fn add_direct_value(&mut self, name: &str, data: &[u8]) -> &mut Self {
        check_state!(!self.frozen);
        self.direct_values.push((name.to_string(), data.to_vec()));
        self
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:320 BuildToolLogCollection.addUri
    pub fn add_uri(&mut self, name: &str, uri: &str) -> &mut Self {
        check_state!(!self.frozen);
        self.future_uris.push((name.to_string(), uri.to_string()));
        self
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:334 BuildToolLogCollection.addLocalFile
    pub fn add_local_file(&mut self, name: &str, path: Path) -> &mut Self {
        self.add_local_file_with(name, path, LocalFileType::LOG, LocalFileCompression::NONE)
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:339 BuildToolLogCollection.addLocalFile
    pub fn add_local_file_with(
        &mut self,
        name: &str,
        path: Path,
        local_file_type: LocalFileType,
        compression: LocalFileCompression,
    ) -> &mut Self {
        self.add_local_file_entry(
            name,
            LocalFile {
                path,
                type_: local_file_type,
                compression,
            },
        )
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:346 BuildToolLogCollection.addLocalFile
    pub fn add_local_file_entry(&mut self, name: &str, local_file: LocalFile) -> &mut Self {
        check_state!(!self.frozen);
        let mut name = name.to_string();
        if local_file.compression == LocalFileCompression::GZIP {
            name.push_str(".gz");
        }
        self.local_files.push(LogFileEntry { name, local_file });
        self
    }

    /// The direct values (name, bytes) in insertion order.
    pub fn get_direct_values(&self) -> &[(String, Vec<u8>)] {
        &self.direct_values
    }

    /// The URIs (name, uri) in insertion order.
    pub fn get_uris(&self) -> &[(String, String)] {
        &self.future_uris
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildResult.java:355 BuildToolLogCollection.toEvent
    pub fn to_event(&self) -> ! {
        check_state!(self.frozen);
        unported!("BuildToolLogs")
    }
}
