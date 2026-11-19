//! Ports of `analysis/AnalysisFailureEvent.java` and `pkgcache/LoadingFailureEvent.java`: the
//! events `SkyframeErrorProcessor` and `SkyframeBuildView.configureTargets` post for a top-level
//! target that failed, with the root causes (`causes/Cause.java`) they carry.
//!
//! PORT: the events are posted on the event handler (`ExtendedEventHandler.post`, Blaze's event
//! bus). `AnalysisFailureEvent` is a `BuildEvent` in Java; its build event protocol half
//! (`getEventId`, `getChildrenEvents`, `asStreamProto`) belongs to `blaze_bep`, which wraps this
//! event.

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_cmdline::Label;
use blaze_events::Postable;
use blaze_protos::failure_details::FailureDetail;
use blaze_sky_analysis::producers::errors::Cause;

/// A root cause of a failed top-level target (Java `Cause`).
// Java: com/google/devtools/build/lib/causes/Cause.java:24 Cause
#[derive(Clone, Debug, PartialEq)]
pub enum AnalysisRootCause {
    /// A cause carried by a `ConfiguredValueCreationException` (`AnalysisFailedCause` or
    /// `LoadingFailedCause`).
    Cause(Cause),
    /// An `AnalysisFailedCause` created for the top-level target.
    // Java: com/google/devtools/build/lib/causes/AnalysisFailedCause.java:30 AnalysisFailedCause
    AnalysisFailed {
        label: Label,
        /// `configurationIdMessage(configurationKey)`: the checksum, or `none`.
        configuration: String,
        /// `getDetailedExitCode().getFailureDetail()`.
        failure_detail: FailureDetail,
    },
    /// Failure due to something associated with a label.
    // Java: com/google/devtools/build/lib/causes/LabelCause.java:24 LabelCause
    Label {
        label: Label,
        /// `getDetailedExitCode().getFailureDetail()`.
        failure_detail: FailureDetail,
    },
}

impl AnalysisRootCause {
    /// Return the label associated with the failure.
    // Java: com/google/devtools/build/lib/causes/Cause.java:27 Cause.getLabel
    pub fn get_label(&self) -> &Label {
        match self {
            AnalysisRootCause::Cause(cause) => cause.get_label(),
            AnalysisRootCause::AnalysisFailed { label, .. }
            | AnalysisRootCause::Label { label, .. } => label,
        }
    }
}

/// This event is fired during the build, when it becomes known that the analysis of a top-level
/// target cannot be completed because of an error in one of its dependencies.
// Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:42 AnalysisFailureEvent
#[derive(Clone, Debug)]
pub struct AnalysisFailureEvent {
    failed_target: ConfiguredTargetKey,
    failed_aspect: Option<AspectKey>,
    /// True if the target is configured.
    ///
    /// The configuration of a target is undefined until its analysis is complete so this is
    /// often false, but true for aspects and action conflict errors, both of which occur after
    /// the configuration is determined.
    is_configured: bool,
    /// `NestedSet<Cause>`, flattened (`toList()`).
    root_causes: Vec<AnalysisRootCause>,
}

impl AnalysisFailureEvent {
    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:57 AnalysisFailureEvent.whileAnalyzingTarget
    pub fn while_analyzing_target(
        failed_target: ConfiguredTargetKey,
        root_causes: Vec<AnalysisRootCause>,
    ) -> Self {
        AnalysisFailureEvent {
            failed_target,
            failed_aspect: None,
            is_configured: false,
            root_causes,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:62 AnalysisFailureEvent.actionConflict
    pub fn action_conflict(
        failed_target: &ActionLookupKeyRef,
        root_causes: Vec<AnalysisRootCause>,
    ) -> Self {
        let as_ct_key = failed_target.as_any().downcast_ref::<ConfiguredTargetKey>();
        let as_aspect_key = failed_target.as_any().downcast_ref::<AspectKey>();
        blaze_util::check_argument!(as_ct_key.is_some() || as_aspect_key.is_some());
        if let Some(ct_key) = as_ct_key {
            return AnalysisFailureEvent {
                failed_target: ct_key.clone(),
                failed_aspect: None,
                is_configured: true,
                root_causes,
            };
        }
        let failed_aspect = as_aspect_key.expect("checked above").clone();
        AnalysisFailureEvent {
            failed_target: failed_aspect.get_base_configured_target_key().clone(),
            failed_aspect: Some(failed_aspect),
            is_configured: true,
            root_causes,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:102 AnalysisFailureEvent.getFailedTarget
    pub fn get_failed_target(&self) -> &ConfiguredTargetKey {
        &self.failed_target
    }

    /// The failed aspect of an action conflict in an aspect (Java's `failedAspect` field, which
    /// `getEventId` reads: `aspectCompleted(label, configurationId, getAspectName())`).
    pub fn get_failed_aspect(&self) -> Option<&AspectKey> {
        self.failed_aspect.as_ref()
    }

    /// Whether the event names a configured target (`getConfigurationId() != null`).
    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:110 AnalysisFailureEvent.getConfigurationId
    pub fn is_configured(&self) -> bool {
        self.is_configured
    }

    /// Returns the label of a single root cause. Use `get_root_causes` to report all root
    /// causes.
    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:117 AnalysisFailureEvent.getLegacyFailureReason
    pub fn get_legacy_failure_reason(&self) -> Option<&Label> {
        self.root_causes.first().map(AnalysisRootCause::get_label)
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:124 AnalysisFailureEvent.getRootCauses
    pub fn get_root_causes(&self) -> &[AnalysisRootCause] {
        &self.root_causes
    }
}

impl Postable for AnalysisFailureEvent {}

/// This event is fired during the build, when it becomes known that the loading of a target
/// cannot be completed because of an error in one of its dependencies.
// Java: com/google/devtools/build/lib/pkgcache/LoadingFailureEvent.java:25 LoadingFailureEvent
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LoadingFailureEvent {
    failed_target: Label,
    failure_reason: Label,
}

impl LoadingFailureEvent {
    // Java: com/google/devtools/build/lib/pkgcache/LoadingFailureEvent.java:29 LoadingFailureEvent.<init>
    pub fn new(failed_target: Label, failure_reason: Label) -> Self {
        LoadingFailureEvent {
            failed_target,
            failure_reason,
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/LoadingFailureEvent.java:42 LoadingFailureEvent.getFailedTarget
    pub fn get_failed_target(&self) -> &Label {
        &self.failed_target
    }

    // Java: com/google/devtools/build/lib/pkgcache/LoadingFailureEvent.java:46 LoadingFailureEvent.getFailureReason
    pub fn get_failure_reason(&self) -> &Label {
        &self.failure_reason
    }
}

impl Postable for LoadingFailureEvent {}
