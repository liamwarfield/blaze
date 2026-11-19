//! Port of `TargetParsingCompleteEvent` (from `lib/pkgcache`).

use blaze_cmdline::label::Label;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};

use crate::buildeventstream::build_event::{BuildEvent, BuildEventClass, Interrupted};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;

/// A target without its package: what the event keeps of a `Target`.
// Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:40 TargetParsingCompleteEvent.ThinTarget
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThinTarget {
    label: Label,
    rule_class: Option<String>,
    target_kind: String,
}

impl ThinTarget {
    /// `rule_class` is `Some` iff the target is a rule.
    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:45 ThinTarget.<init>
    pub fn new(label: Label, target_kind: String, rule_class: Option<String>) -> Self {
        ThinTarget {
            label,
            rule_class,
            target_kind,
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:51 ThinTarget.isRule
    pub fn is_rule(&self) -> bool {
        self.rule_class.is_some()
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:55 ThinTarget.getTargetKind
    pub fn get_target_kind(&self) -> &str {
        &self.target_kind
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:59 ThinTarget.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:64 ThinTarget.getRuleClass
    pub fn get_rule_class(&self) -> &str {
        self.rule_class
            .as_deref()
            .unwrap_or_else(|| panic!("NullPointerException: {}", self.label))
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:68 ThinTarget.isTestSuiteRule
    pub fn is_test_suite_rule(&self) -> bool {
        // TargetUtils.isTestSuiteRuleName: "test_suite".equals(name)
        self.is_rule() && self.get_rule_class() == "test_suite"
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:72 ThinTarget.isNotATestOrTestSuite
    pub fn is_not_a_test_or_test_suite(&self) -> bool {
        // TargetUtils.isTestRuleName: name.endsWith("_test")
        !self.is_rule() || (!self.is_test_suite_rule() && !self.get_rule_class().ends_with("_test"))
    }
}

/// This event is fired just after target pattern evaluation is completed.
// Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:38 TargetParsingCompleteEvent
#[derive(Clone, Debug)]
pub struct TargetParsingCompleteEvent {
    original_target_pattern: Vec<String>,
    failed_target_patterns: Vec<String>,
    targets: Vec<ThinTarget>,
    filtered_targets: Vec<ThinTarget>,
    test_filtered_targets: Vec<ThinTarget>,
    expanded_targets: Vec<ThinTarget>,
    /// `ImmutableSetMultimap<String, Label>` in insertion order.
    original_patterns_to_labels: Vec<(String, Label)>,
    /// `ImmutableMap<Label, ImmutableSet<Label>>` in insertion order.
    test_suite_expansions: Vec<(Label, Vec<Label>)>,
}

fn as_thin_targets(targets: Vec<ThinTarget>) -> Vec<ThinTarget> {
    // ImmutableSet: first occurrence wins.
    let mut out: Vec<ThinTarget> = Vec::with_capacity(targets.len());
    for t in targets {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

impl TargetParsingCompleteEvent {
    /// Construct the event.
    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:86 TargetParsingCompleteEvent.<init>
    pub fn new(
        targets: Vec<ThinTarget>,
        filtered_targets: Vec<ThinTarget>,
        test_filtered_targets: Vec<ThinTarget>,
        original_target_pattern: Vec<String>,
        expanded_targets: Vec<ThinTarget>,
        failed_target_patterns: Vec<String>,
        original_patterns_to_labels: Vec<(String, Label)>,
        test_suite_expansions: Vec<(Label, Vec<Label>)>,
    ) -> Self {
        TargetParsingCompleteEvent {
            targets: as_thin_targets(targets),
            filtered_targets: as_thin_targets(filtered_targets),
            test_filtered_targets: as_thin_targets(test_filtered_targets),
            original_target_pattern,
            expanded_targets: as_thin_targets(expanded_targets),
            failed_target_patterns,
            original_patterns_to_labels,
            test_suite_expansions,
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:105 TargetParsingCompleteEvent.getOriginalTargetPattern
    pub fn get_original_target_pattern(&self) -> &[String] {
        &self.original_target_pattern
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:109 TargetParsingCompleteEvent.getFailedTargetPatterns
    pub fn get_failed_target_patterns(&self) -> &[String] {
        &self.failed_target_patterns
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:114 TargetParsingCompleteEvent.getTargets
    pub fn get_targets(&self) -> &[ThinTarget] {
        &self.targets
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:131 TargetParsingCompleteEvent.getFilteredTargets
    pub fn get_filtered_targets(&self) -> &[ThinTarget] {
        &self.filtered_targets
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:136 TargetParsingCompleteEvent.getTestFilteredTargets
    pub fn get_test_filtered_targets(&self) -> &[ThinTarget] {
        &self.test_filtered_targets
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:150 TargetParsingCompleteEvent.getOriginalPatternsToLabels
    pub fn get_original_patterns_to_labels(&self) -> &[(String, Label)] {
        &self.original_patterns_to_labels
    }
}

impl BuildEvent for TargetParsingCompleteEvent {
    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:155 TargetParsingCompleteEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::target_pattern_expanded(&self.original_target_pattern)
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:160 TargetParsingCompleteEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_started_id()])
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:165 TargetParsingCompleteEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        let mut children_builder = Vec::new();
        for failed_target_pattern in &self.failed_target_patterns {
            children_builder.push(build_event_id_util::target_pattern_expanded(
                std::slice::from_ref(failed_target_pattern),
            ));
        }
        for target in &self.expanded_targets {
            if !target.is_test_suite_rule() {
                children_builder.push(build_event_id_util::target_configured(target.get_label()));
            }
        }
        children_builder
    }

    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:182 TargetParsingCompleteEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut expanded = bes::PatternExpanded::default();
        for (suite, tests) in &self.test_suite_expansions {
            expanded
                .test_suite_expansions
                .push(bes::pattern_expanded::TestSuiteExpansion {
                    suite_label: suite.to_string(),
                    test_labels: tests.iter().map(Label::to_string).collect(),
                });
        }
        Ok(proto_chaining_with(self, Payload::Expanded(expanded)))
    }

    fn event_class(&self) -> BuildEventClass {
        BuildEventClass::TargetParsingComplete {
            original_target_pattern_count: self.original_target_pattern.len(),
            failed_target_patterns_empty: self.failed_target_patterns.is_empty(),
        }
    }
}

impl blaze_events::Postable for TargetParsingCompleteEvent {
    // Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:195 TargetParsingCompleteEvent.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}

/// The build event half of `PatternExpandingError` (the event itself is posted by
/// `blaze_sky_top`): an `Aborted` with reason `LOADING_FAILURE` for the pattern.
// Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:27 PatternExpandingError
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternExpandingErrorEvent {
    pattern: Vec<String>,
    message: String,
    skipped: bool,
}

impl PatternExpandingErrorEvent {
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:33 PatternExpandingError.<init>
    pub fn new(pattern: Vec<String>, message: String, skipped: bool) -> Self {
        PatternExpandingErrorEvent {
            pattern,
            message,
            skipped,
        }
    }
}

impl BuildEvent for PatternExpandingErrorEvent {
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:58 PatternExpandingError.getEventId
    fn get_event_id(&self) -> BuildEventId {
        if self.skipped {
            build_event_id_util::target_pattern_skipped(&self.pattern)
        } else {
            build_event_id_util::target_pattern_expanded(&self.pattern)
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:67 PatternExpandingError.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:72 PatternExpandingError.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let failure = bes::Aborted {
            reason: bes::aborted::AbortReason::LoadingFailure as i32,
            description: self.message.clone(),
        };
        Ok(proto_chaining_with(self, Payload::Aborted(failure)))
    }
}

impl blaze_events::Postable for PatternExpandingErrorEvent {
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:83 PatternExpandingError.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}
