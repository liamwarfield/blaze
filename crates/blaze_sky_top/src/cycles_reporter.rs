//! Ports of `skyframe/CyclesReporter.java`, `skyframe/CycleDeduper.java`,
//! `lib/skyframe/AbstractLabelCycleReporter.java` and `lib/skyframe/TargetCycleReporter.java`.
//!
//! PORT: `CyclesReporter` and `CycleDeduper` belong to the `skyframe` package
//! (`blaze_skyframe`); they are kept here, next to the only reporters that exist, until that
//! crate takes them.
//!
//! Java's `SkyframeExecutor.createCyclesReporter` registers seven reporters, all ported (see
//! `SkyframeExecutor::new`); `TestExpansionCycleReporter` is here. A cycle none of them reports
//! gets Java's "could not be properly displayed" error.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_analysis::aspect_key_creator::{
    ASPECT, AspectKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey,
};
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::target::{PackageGroup, Target};
use blaze_sky_loading::package_manager::{NoSuchPackageOrInterrupted, SkyframePackageManager};
use blaze_sky_loading::transitive_target::{TRANSITIVE_TARGET, TransitiveTargetKey};
use blaze_skyframe::{CycleInfo, SkyFunctionName, SkyKey};
use blaze_util::{check_argument, check_state};

use crate::target_pattern_phase::TestExpansionKey;

/// Interface for reporting custom information about a single cycle.
// Java: com/google/devtools/build/skyframe/CyclesReporter.java:38 CyclesReporter.SingleCycleReporter
pub trait SingleCycleReporter: Send + Sync {
    /// Reports the given cycle and returns `true`, or returns `false` if this
    /// `SingleCycleReporter` doesn't know how to report the cycle.
    ///
    /// - `top_level_key`: the top level key that transitively depended on the cycle
    /// - `cycle_info`: the cycle
    /// - `already_reported`: whether the cycle has already been reported to the
    ///   [`CyclesReporter`].
    /// - `event_handler`: the event handler to which to report the error
    // Java: com/google/devtools/build/skyframe/CyclesReporter.java:50 CyclesReporter.SingleCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool;
}

/// An utility for custom reporting of errors from cycles in the Skyframe graph. This class is
/// stateful in order to differentiate between new cycles and cycles that have already been
/// reported (do not reuse the instances or cache the results as it could end up printing
/// inconsistent information or leak memory). It treats two cycles as the same if they contain
/// the same `SkyKey`s in the same order, but perhaps with different starting points. See
/// [`CycleDeduper`] for more information.
// Java: com/google/devtools/build/skyframe/CyclesReporter.java:31 CyclesReporter
pub struct CyclesReporter {
    cycle_reporters: Vec<Box<dyn SingleCycleReporter>>,
    cycle_deduper: parking_lot::Mutex<CycleDeduper>,
}

impl CyclesReporter {
    /// Constructs a `CyclesReporter` that delegates to the given `SingleCycleReporter`s, in the
    /// given order, to report custom information about cycles.
    // Java: com/google/devtools/build/skyframe/CyclesReporter.java:64 CyclesReporter.<init>
    pub fn new(cycle_reporters: Vec<Box<dyn SingleCycleReporter>>) -> Self {
        CyclesReporter {
            cycle_reporters,
            cycle_deduper: parking_lot::Mutex::new(CycleDeduper::new()),
        }
    }

    /// Reports the given cycles, differentiating between cycles that have already been reported.
    ///
    /// - `cycles`: the cycles.
    /// - `top_level_key`: this key represents the top level value key that returned cycle errors.
    /// - `event_handler`: the event handler to which to report the error
    // Java: com/google/devtools/build/skyframe/CyclesReporter.java:75 CyclesReporter.reportCycles
    pub fn report_cycles(
        &self,
        cycles: &[CycleInfo],
        top_level_key: &SkyKey,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) {
        let mut first_cycle = true;
        for cycle_info in cycles {
            // TODO(janakr): if this assertion is never hit, remove topLevelKey as an argument to
            // method.
            // PORT: skipped, BugReport.sendNonFatalBugReport when the cycle does not start with
            // the top-level key (bug reports), and the "Some cycles were omitted" log line
            // (logging).
            self.maybe_report_cycle(cycle_info, top_level_key, first_cycle, event_handler);
            first_cycle = false;
        }
    }

    /// Returns true if it reported the cycle.
    // Java: com/google/devtools/build/skyframe/CyclesReporter.java:96 CyclesReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        cycle_info: &CycleInfo,
        top_level_key: &SkyKey,
        first_cycle: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        let already_reported = self
            .cycle_deduper
            .lock()
            .already_seen(cycle_info.get_cycle());
        if !first_cycle && already_reported {
            // We've already reported this top-level key and this cycle, although maybe not
            // together. Enumerating all combinations of top-level keys and cycles is too spammy
            // for the user.
            return false;
        }
        for cycle_reporter in &self.cycle_reporters {
            if cycle_reporter.maybe_report_cycle(
                top_level_key,
                cycle_info,
                already_reported,
                event_handler,
            ) {
                return true;
            }
        }

        // No proper cycle reporter could be found. Blaze bug! Not fatal, though.
        let raw_cycle = print_arbitrary_cycle(top_level_key, cycle_info, already_reported);
        event_handler.handle(&Event::error(format!(
            "Cycle detected but could not be properly displayed due to an internal problem. \
             Please file an issue. Raw display: {raw_cycle}"
        )));
        // PORT: Java appends the reporters' `Object.toString` (class name and identity hash code)
        // to the bug report; Blaze names the reporters only by their count.
        blaze_util::bug_report::send_bug_report(&format!(
            "{raw_cycle}\n[{} cycle reporters]",
            self.cycle_reporters.len()
        ));
        true
    }
}

// Java: com/google/devtools/build/skyframe/CyclesReporter.java:124 CyclesReporter.printArbitraryCycle
fn print_arbitrary_cycle(
    top_level_key: &SkyKey,
    cycle_info: &CycleInfo,
    already_reported: bool,
) -> String {
    let mut cycle_message = format!(
        "topLevelKey: {:?}\nalreadyReported: {}\npath to cycle:\n",
        top_level_key, already_reported
    );
    for sky_key in cycle_info.get_path_to_cycle() {
        cycle_message.push_str(&format!("{:?}\n", sky_key));
    }
    cycle_message.push_str("cycle:\n");
    for sky_key in cycle_info.get_cycle() {
        cycle_message.push_str(&format!("{:?}\n", sky_key));
    }
    cycle_message
}

/// Dedupes C candidate cycles of size O(L) in O(CL) time and memory in the common case and
/// O(C^2 * L) time and O(CL) memory in the extreme case.
///
/// Two cycles are considered duplicates if they are exactly the same except for the entry point.
/// For example, 'a' -> 'b' -> 'c' -> 'a' is the considered the same as 'b' -> 'c' -> 'a' -> 'b'.
///
/// PORT: Java keys a `HashMultimap` by the set of members; a set of `SkyKey`s has no hash here,
/// so the member sets are searched linearly (same answers).
// Java: com/google/devtools/build/skyframe/CycleDeduper.java:30 CycleDeduper
pub struct CycleDeduper {
    known_cycles_by_members: Vec<(HashSet<SkyKey>, Vec<Vec<SkyKey>>)>,
}

impl CycleDeduper {
    pub fn new() -> Self {
        CycleDeduper {
            known_cycles_by_members: Vec::new(),
        }
    }

    /// Marks a non-empty list representing a cycle of unique values as being seen and returns
    /// true iff the cycle has been seen before, accounting for logical equivalence of cycles.
    ///
    /// For example, the cycle 'a' -> 'b' -> 'c' -> 'a' is represented by the list ['a', 'b',
    /// 'c'] and is logically equivalent to the cycle represented by the list ['b', 'c', 'a'].
    // Java: com/google/devtools/build/skyframe/CycleDeduper.java:41 CycleDeduper.alreadySeen
    pub fn already_seen(&mut self, cycle: &[SkyKey]) -> bool {
        let cycle_members: HashSet<SkyKey> = cycle.iter().cloned().collect();
        check_state!(!cycle.is_empty());
        check_state!(
            cycle.len() == cycle_members.len(),
            "cycle doesn't have unique members: {:?}",
            cycle
        );

        let index = match self
            .known_cycles_by_members
            .iter()
            .position(|(members, _)| *members == cycle_members)
        {
            Some(index) => index,
            None => {
                self.known_cycles_by_members
                    .push((cycle_members, Vec::new()));
                self.known_cycles_by_members.len() - 1
            }
        };
        let known_cycles = &mut self.known_cycles_by_members[index].1;
        if known_cycles.iter().any(|known| known.as_slice() == cycle) {
            return true;
        }

        // Of the C cycles, suppose there are D cycles that have the same members (but are in an
        // incompatible order). This code path takes O(D * L) time. The common case is that D is
        // very small.
        let mut found = false;
        for candidate_cycle in known_cycles.iter() {
            let start_pos = candidate_cycle.iter().position(|k| *k == cycle[0]);
            // The use of a multimap keyed by cycle members guarantees that the first element of
            // 'cycle' is present in 'candidateCycle'.
            let start_pos = start_pos.unwrap_or_else(|| panic!("IllegalStateException"));
            if equals_with_single_loop_from(cycle, candidate_cycle, start_pos) {
                found = true;
                break;
            }
        }
        // We add the cycle even if it's a duplicate so that future exact copies of this can be
        // processed in O(L) time. We are already using O(CL) memory, and this optimization
        // doesn't change that.
        known_cycles.push(cycle.to_vec());
        found
    }
}

/// Returns true iff
///   listA[0], listA[1], ..., listA[listA.size()]
/// is the same as
///   listB[start], listB[start+1], ..., listB[listB.size()-1], listB[0], ..., listB[start-1]
// Java: com/google/devtools/build/skyframe/CycleDeduper.java:77 CycleDeduper.equalsWithSingleLoopFrom
fn equals_with_single_loop_from(list_a: &[SkyKey], list_b: &[SkyKey], start: usize) -> bool {
    if list_a.len() != list_b.len() {
        return false;
    }
    let length = list_a.len();
    for i in 0..length {
        if list_a[i] != list_b[(i + start) % length] {
            return false;
        }
    }
    true
}

/// Prints the SkyKey-s in cycle into `cycle_message` using the print function. Returns the first
/// key of the cycle.
// Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:146 AbstractLabelCycleReporter.printCycle
pub fn print_cycle(
    cycle: &[SkyKey],
    cycle_message: &mut String,
    print_function: &dyn Fn(&SkyKey) -> String,
    should_skip_intermediate_key: &dyn Fn(&SkyKey) -> bool,
) -> SkyKey {
    check_argument!(!cycle.is_empty());
    let mut cycle_value: Option<&SkyKey> = None;
    let mut values_printed = 0;
    let count = cycle.len() + 1;
    for i in 0..count {
        // Iterables.concat(cycle, ImmutableList.of(cycle.get(0)))
        let value = if i < cycle.len() {
            &cycle[i]
        } else {
            &cycle[0]
        };
        match cycle_value {
            None => {
                // first item
                cycle_value = Some(value);
                cycle_message.push_str("\n.-> ");
            }
            // PORT: Java compares by identity (`value == cycleValue`); the members of a cycle
            // are unique, so only the appended first key is identical to it.
            Some(_) if i == cycle.len() => {
                // last item of the cycle
                if values_printed == 1 {
                    cycle_message.push_str(" [self-edge]");
                    cycle_message.push_str("\n`--");
                    break;
                } else {
                    cycle_message.push_str("\n`-- ");
                }
            }
            Some(_) => {
                if should_skip_intermediate_key(value) {
                    continue;
                } else {
                    cycle_message.push_str("\n|   ");
                }
            }
        }
        cycle_message.push_str(&print_function(value));
        values_printed += 1;
    }

    cycle_value.expect("non-empty cycle").clone()
}

/// Reports cycles between skyframe values whose keys contain `Label`s (for example, targets
/// and actions).
///
/// PORT: Java's abstract base class; its template method `maybeReportCycle` is
/// [`label_maybe_report_cycle`], which each subclass's `SingleCycleReporter` implementation calls.
// Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:38 AbstractLabelCycleReporter
pub trait AbstractLabelCycleReporter {
    fn package_provider(&self) -> &Arc<SkyframePackageManager>;

    /// Returns the associated Label of the SkyKey.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:47 AbstractLabelCycleReporter.getLabel
    fn get_label(&self, key: &SkyKey) -> Label;

    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:49 AbstractLabelCycleReporter.canReportCycle
    fn can_report_cycle(&self, top_level_key: &SkyKey, cycle_info: &CycleInfo) -> bool;

    /// Returns the String representation of the `SkyKey`.
    ///
    /// PORT: both subclasses override Java's default (the label of an `ActionLookupKey`, else
    /// `getLabel(key)`), so it is required here.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:52 AbstractLabelCycleReporter.prettyPrint
    fn pretty_print(&self, key: &SkyKey) -> String;

    /// Can be used to skip individual keys on the path to the cycle.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:60 AbstractLabelCycleReporter.shouldSkipOnPathToCycle
    fn should_skip_on_path_to_cycle(&self, _key: &SkyKey) -> bool {
        false
    }

    /// Can be used to skip intermediate keys on the cycle itself.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:65 AbstractLabelCycleReporter.shouldSkipIntermediateKeyOnCycle
    fn should_skip_intermediate_key_on_cycle(&self, _key: &SkyKey) -> bool {
        false
    }

    /// Can be used to report an additional message about the cycle.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:76 AbstractLabelCycleReporter.getAdditionalMessageAboutCycle
    fn get_additional_message_about_cycle(
        &self,
        _event_handler: &Arc<dyn ExtendedEventHandler>,
        _top_level_key: &SkyKey,
        _cycle_info: &CycleInfo,
    ) -> String {
        String::new()
    }

    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:174 AbstractLabelCycleReporter.getTargetForLabel
    fn get_target_for_label(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Target {
        // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:36 TargetProvider.getTarget
        let package = match self
            .package_provider()
            .get_package(event_handler.clone(), label.get_package_identifier())
        {
            Ok(package) => package,
            // This method is used for getting the target from a label in a circular dependency.
            // If we have a cycle that means that we need to have accessed the target (to get its
            // dependencies). So all the labels in a dependency cycle need to exist.
            Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => {
                panic!("IllegalStateException: {}", e)
            }
            Err(NoSuchPackageOrInterrupted::Interrupted) => {
                panic!("IllegalStateException: java.lang.InterruptedException")
            }
        };
        match package.get_target(label.get_name()) {
            Ok(target) => target.clone(),
            Err(e) => panic!("IllegalStateException: {}", e),
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:83 AbstractLabelCycleReporter.maybeReportCycle
pub fn label_maybe_report_cycle(
    reporter: &dyn AbstractLabelCycleReporter,
    top_level_key: &SkyKey,
    cycle_info: &CycleInfo,
    already_reported: bool,
    event_handler: &Arc<dyn ExtendedEventHandler>,
) -> bool {
    if !reporter.can_report_cycle(top_level_key, cycle_info) {
        return false;
    }

    if already_reported {
        if !reporter.should_skip_on_path_to_cycle(top_level_key) {
            let label = reporter.get_label(top_level_key);
            let target = reporter.get_target_for_label(event_handler, &label);
            event_handler.handle(&Event::error_at(
                Some(blaze_sky_loading::starlark_util::event_location(
                    &target.get_location(),
                )),
                format!(
                    "in {} {}: cycle in dependency graph: target depends on an \
                     already-reported cycle",
                    target.get_target_kind(),
                    label
                ),
            ));
        }
    } else {
        let mut cycle_message = String::from("cycle in dependency graph:");
        let path_to_cycle = cycle_info.get_path_to_cycle();
        let cycle = cycle_info.get_cycle();
        for value in path_to_cycle {
            if reporter.should_skip_on_path_to_cycle(value) {
                continue;
            }
            cycle_message.push_str("\n    ");
            cycle_message.push_str(&reporter.pretty_print(value));
        }

        let cycle_value = print_cycle(
            cycle,
            &mut cycle_message,
            &|key| reporter.pretty_print(key),
            &|key| reporter.should_skip_intermediate_key_on_cycle(key),
        );

        cycle_message.push_str(&reporter.get_additional_message_about_cycle(
            event_handler,
            top_level_key,
            cycle_info,
        ));

        let label = reporter.get_label(&cycle_value);
        let target = reporter.get_target_for_label(event_handler, &label);
        event_handler.handle(&Event::error_at(
            Some(blaze_sky_loading::starlark_util::event_location(
                &target.get_location(),
            )),
            format!(
                "in {} {}: {}",
                target.get_target_kind(),
                label,
                cycle_message
            ),
        ));
    }

    true
}

/// Reports cycles occurring in during the expansion of `test_suite` rules.
// Java: com/google/devtools/build/lib/skyframe/TestExpansionCycleReporter.java:23 TestExpansionCycleReporter
pub struct TestExpansionCycleReporter {
    package_provider: Arc<SkyframePackageManager>,
}

impl TestExpansionCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/TestExpansionCycleReporter.java:25 TestExpansionCycleReporter.<init>
    pub fn new(package_provider: Arc<SkyframePackageManager>) -> Self {
        TestExpansionCycleReporter { package_provider }
    }
}

impl AbstractLabelCycleReporter for TestExpansionCycleReporter {
    fn package_provider(&self) -> &Arc<SkyframePackageManager> {
        &self.package_provider
    }

    // Java: com/google/devtools/build/lib/skyframe/TestExpansionCycleReporter.java:30 TestExpansionCycleReporter.canReportCycle
    fn can_report_cycle(&self, _top_level_key: &SkyKey, cycle_info: &CycleInfo) -> bool {
        cycle_info
            .get_cycle()
            .iter()
            .all(|key| key.downcast::<TestExpansionKey>().is_some())
    }

    // Java: com/google/devtools/build/lib/skyframe/TestExpansionCycleReporter.java:35 TestExpansionCycleReporter.shouldSkipOnPathToCycle
    fn should_skip_on_path_to_cycle(&self, key: &SkyKey) -> bool {
        key.downcast::<TestExpansionKey>().is_none()
    }

    /// Java's default `prettyPrint`: the label of an `ActionLookupKey`, else `getLabel(key)`.
    /// PORT: the keys this reporter prints are `TestExpansionKey`s (the cycle is made of them and
    /// every other key on the path is skipped), never `ActionLookupKey`s.
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:52 AbstractLabelCycleReporter.prettyPrint
    fn pretty_print(&self, key: &SkyKey) -> String {
        self.get_label(key).to_string()
    }

    // Java: com/google/devtools/build/lib/skyframe/TestExpansionCycleReporter.java:40 TestExpansionCycleReporter.getLabel
    fn get_label(&self, key: &SkyKey) -> Label {
        match key.downcast::<TestExpansionKey>() {
            Some(test_expansion_key) => test_expansion_key.get_label().clone(),
            // Java: ClassCastException.
            None => panic!("ClassCastException: {:?}", key),
        }
    }
}

impl SingleCycleReporter for TestExpansionCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/AbstractLabelCycleReporter.java:83 AbstractLabelCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        label_maybe_report_cycle(
            self,
            top_level_key,
            cycle_info,
            already_reported,
            event_handler,
        )
    }
}

/// Reports cycles between targets. These may be in the form of `ConfiguredTargetValue`s or
/// `TransitiveTargetValue`s.
///
// Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:36 TargetCycleReporter
pub struct TargetCycleReporter {
    package_provider: Arc<SkyframePackageManager>,
}

/// `SkyFunctions.PREPARE_ANALYSIS_PHASE`.
const PREPARE_ANALYSIS_PHASE: SkyFunctionName = SkyFunctionName("PREPARE_ANALYSIS_PHASE");
/// `SkyFunctions.BUILD_DRIVER`.
const BUILD_DRIVER: SkyFunctionName = SkyFunctionName("BUILD_DRIVER");

// Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:38 TargetCycleReporter.CONFIGURED_TARGET_OR_TRANSITIVE_RDEP
fn configured_target_or_transitive_rdep(key: &SkyKey) -> bool {
    let name = key.function_name();
    name == CONFIGURED_TARGET
        || name == ASPECT
        || name == TOP_LEVEL_ASPECTS
        || name == TRANSITIVE_TARGET
        || name == PREPARE_ANALYSIS_PHASE
        || name == BUILD_DRIVER
}

impl TargetCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:47 TargetCycleReporter.<init>
    pub fn new(package_provider: Arc<SkyframePackageManager>) -> Self {
        TargetCycleReporter { package_provider }
    }
}

impl AbstractLabelCycleReporter for TargetCycleReporter {
    fn package_provider(&self) -> &Arc<SkyframePackageManager> {
        &self.package_provider
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:52 TargetCycleReporter.shouldSkipOnPathToCycle
    fn should_skip_on_path_to_cycle(&self, key: &SkyKey) -> bool {
        PREPARE_ANALYSIS_PHASE == key.function_name()
            // BuildDriverKeys don't provide any relevant info for the end user.
            || BUILD_DRIVER == key.function_name()
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:59 TargetCycleReporter.canReportCycle
    fn can_report_cycle(&self, top_level_key: &SkyKey, cycle_info: &CycleInfo) -> bool {
        configured_target_or_transitive_rdep(top_level_key)
            && cycle_info
                .get_path_to_cycle()
                .iter()
                .all(configured_target_or_transitive_rdep)
            && cycle_info
                .get_cycle()
                .iter()
                .all(configured_target_or_transitive_rdep)
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:66 TargetCycleReporter.prettyPrint
    fn pretty_print(&self, key: &SkyKey) -> String {
        if let Some(configured_target_key) = key.downcast::<ConfiguredTargetKey>() {
            configured_target_key.pretty_print()
        } else if let Some(aspect_key) = key.downcast::<AspectKey>() {
            aspect_key.pretty_print()
        } else {
            self.get_label(key).to_string()
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:77 TargetCycleReporter.getLabel
    fn get_label(&self, key: &SkyKey) -> Label {
        // key instanceof ActionLookupKey: the configured target and aspect keys of this
        // reporter (Preconditions.checkNotNull: their labels are never null).
        if let Some(configured_target_key) = key.downcast::<ConfiguredTargetKey>() {
            configured_target_key.get_label().clone()
        } else if let Some(aspect_key) = key.downcast::<AspectKey>() {
            aspect_key.get_label().clone()
        } else if let Some(top_level_aspects_key) = key.downcast::<TopLevelAspectsKey>() {
            top_level_aspects_key.get_label().clone()
        } else if let Some(transitive_target_key) = key.downcast::<TransitiveTargetKey>() {
            transitive_target_key.get_label().clone()
        } else {
            panic!("UnsupportedOperationException: {:?}", key)
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCycleReporter.java:88 TargetCycleReporter.getAdditionalMessageAboutCycle
    fn get_additional_message_about_cycle(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
    ) -> String {
        let mut keys: Vec<SkyKey> = Vec::new();
        if !cycle_info.get_path_to_cycle().is_empty() {
            if !self.should_skip_on_path_to_cycle(top_level_key) {
                keys.push(top_level_key.clone());
            }
            cycle_info
                .get_path_to_cycle()
                .iter()
                .filter(|key| !self.should_skip_on_path_to_cycle(key))
                .for_each(|key| keys.push(key.clone()));
        }
        keys.extend(cycle_info.get_cycle().iter().cloned());
        // Make sure we check the edge from the last element of the cycle to the first element of
        // the cycle.
        keys.push(cycle_info.get_cycle()[0].clone());

        let mut current_target =
            self.get_target_for_label(event_handler, &self.get_label(&keys[0]));
        for next_key in &keys {
            let next_label = self.get_label(next_key);
            let next_target = self.get_target_for_label(event_handler, &next_label);
            // TODO(aranguyen): remove this code as a result of b/128716030
            // This is inefficient but it's no big deal since we only do this when there's a
            // cycle.
            if next_target.get_target_kind() != PackageGroup::target_kind()
                && current_target
                    .get_visibility_dependency_labels()
                    .contains(&next_label)
            {
                return format!(
                    "\nThe cycle is caused by a visibility edge from {} to the non-package_group \
                     target {}. Note that visibility labels are supposed to be package_group \
                     targets, which prevents cycles of this form.",
                    current_target.get_label(),
                    next_target.get_label()
                );
            }
            current_target = next_target;
        }
        String::new()
    }
}

impl SingleCycleReporter for TargetCycleReporter {
    fn maybe_report_cycle(
        &self,
        top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        label_maybe_report_cycle(
            self,
            top_level_key,
            cycle_info,
            already_reported,
            event_handler,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_skyframe::Key;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct TestKey(&'static str);

    impl Key for TestKey {
        type Value = ();
        const FUNCTION: SkyFunctionName = SkyFunctionName("TEST_CYCLE_KEY");
        // test key; hermetic like Java's SkyFunctionName.FOR_TESTING
        const HERMETICITY: blaze_skyframe::FunctionHermeticity =
            blaze_skyframe::FunctionHermeticity::Hermetic;
    }

    fn keys(names: &[&'static str]) -> Vec<SkyKey> {
        names.iter().map(|n| SkyKey::new(TestKey(n))).collect()
    }

    /// A `SingleCycleReporter` that records its calls (the tests' lambdas and Mockito mock).
    struct RecordingReporter {
        result: bool,
        calls: Arc<parking_lot::Mutex<Vec<(SkyKey, CycleInfo, bool)>>>,
    }

    impl SingleCycleReporter for RecordingReporter {
        fn maybe_report_cycle(
            &self,
            top_level_key: &SkyKey,
            cycle_info: &CycleInfo,
            already_reported: bool,
            _event_handler: &Arc<dyn ExtendedEventHandler>,
        ) -> bool {
            self.calls
                .lock()
                .push((top_level_key.clone(), cycle_info.clone(), already_reported));
            self.result
        }
    }

    fn recording(
        result: bool,
    ) -> (
        Box<dyn SingleCycleReporter>,
        Arc<parking_lot::Mutex<Vec<(SkyKey, CycleInfo, bool)>>>,
    ) {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        (
            Box::new(RecordingReporter {
                result,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    fn null_event_handler() -> Arc<dyn ExtendedEventHandler> {
        Arc::new(blaze_events::StoredEventHandler::new())
    }

    fn key(name: &'static str) -> SkyKey {
        SkyKey::new(TestKey(name))
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CyclesReporterTest.java:39 CyclesReporterTest.nullEventHandler
    #[test]
    #[ignore = "Rust has no null event handler: the parameter is a reference"]
    fn null_event_handler_test() {}

    // Java: src/test/java/com/google/devtools/build/skyframe/CyclesReporterTest.java:50 CyclesReporterTest.notReportedAssertion
    #[test]
    #[ignore = "the bug report rethrows only in a test environment (TestType.isInTest: TEST_TMPDIR set \
                when the process starts); run the test binary with TEST_TMPDIR set and --ignored \
                (scripts/cargo-capped does not pass TEST_TMPDIR)"]
    #[should_panic(expected = "java.lang.IllegalStateException")]
    fn not_reported_assertion() {
        let (single_reporter, _) = recording(false);
        let cycle_info = CycleInfo::create_cycle_info_without_path(vec![key("func")]);
        let cycles_reporter = CyclesReporter::new(vec![single_reporter]);
        cycles_reporter.report_cycles(&[cycle_info], &key("func"), &null_event_handler());
    }

    /// Not a Java test: what `notReportedAssertion` sees before the bug report, the error event.
    #[test]
    fn not_reported_cycle_is_an_error_event() {
        if blaze_util::test_type::TestType::is_in_test() {
            return; // the bug report panics (see `not_reported_assertion`)
        }
        let (single_reporter, _) = recording(false);
        let cycle_info = CycleInfo::create_cycle_info_without_path(vec![key("func")]);
        let cycles_reporter = CyclesReporter::new(vec![single_reporter]);
        let stored = Arc::new(blaze_events::StoredEventHandler::new());
        let handler: Arc<dyn ExtendedEventHandler> = stored.clone();
        cycles_reporter.report_cycles(&[cycle_info], &key("func"), &handler);
        let events = stored.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].get_message(),
            "Cycle detected but could not be properly displayed due to an internal problem. \
             Please file an issue. Raw display: topLevelKey: TestKey(\"func\")\n\
             alreadyReported: false\npath to cycle:\ncycle:\nTestKey(\"func\")\n"
        );
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CyclesReporterTest.java:64 CyclesReporterTest.smoke
    #[test]
    fn smoke() {
        let (single_reporter, calls) = recording(true);
        let cycle_info = CycleInfo::create_cycle_info_without_path(vec![key("func")]);
        let cycles_reporter = CyclesReporter::new(vec![single_reporter]);
        cycles_reporter.report_cycles(&[cycle_info], &key("func"), &null_event_handler());
        assert!(!calls.lock().is_empty());
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CyclesReporterTest.java:79 CyclesReporterTest.alreadyReportedCycles
    #[test]
    fn already_reported_cycles() {
        let (mock_reporter, calls) = recording(true);
        let cycles_reporter = CyclesReporter::new(vec![mock_reporter]);
        let (top1, top2) = (key("top1"), key("top2"));
        let (path1, path2) = (key("path1"), key("path2"));
        let (cycle1, cycle2) = (key("cycle1"), key("cycle2"));
        let cycles_for = |top: &SkyKey| {
            vec![
                CycleInfo::create_cycle_info(
                    vec![top.clone(), path1.clone()],
                    vec![cycle1.clone(), cycle2.clone()],
                ),
                CycleInfo::create_cycle_info(
                    vec![top.clone(), path2.clone()],
                    vec![cycle1.clone(), cycle2.clone()],
                ),
                CycleInfo::create_cycle_info(
                    vec![top.clone(), path1.clone()],
                    vec![cycle2.clone(), cycle1.clone()],
                ),
                CycleInfo::create_cycle_info(
                    vec![top.clone(), path2.clone()],
                    vec![cycle2.clone(), cycle1.clone()],
                ),
            ]
        };
        let top1_cycles = cycles_for(&top1);
        cycles_reporter.report_cycles(&top1_cycles, &top1, &null_event_handler());
        {
            let calls = calls.lock();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, top1);
            assert_eq!(
                calls[0].1.get_path_to_cycle(),
                top1_cycles[0].get_path_to_cycle()
            );
            assert_eq!(calls[0].1.get_cycle(), top1_cycles[0].get_cycle());
            assert!(!calls[0].2);
        }
        // Second cycle is filtered out because it is equivalent but for the path and cycle order.

        let top2_cycles = cycles_for(&top2);
        cycles_reporter.report_cycles(&top2_cycles, &top2, &null_event_handler());
        let calls = calls.lock();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, top2);
        assert_eq!(
            calls[1].1.get_path_to_cycle(),
            top2_cycles[0].get_path_to_cycle()
        );
        assert_eq!(calls[1].1.get_cycle(), top2_cycles[0].get_cycle());
        assert!(calls[1].2);
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CycleDeduperTest.java:31 CycleDeduperTest.simple
    #[test]
    fn simple() {
        let mut cycle_deduper = CycleDeduper::new();
        assert!(!cycle_deduper.already_seen(&keys(&["a", "b"])));
        assert!(cycle_deduper.already_seen(&keys(&["a", "b"])));
        assert!(cycle_deduper.already_seen(&keys(&["b", "a"])));

        assert!(!cycle_deduper.already_seen(&keys(&["a", "b", "c"])));
        assert!(cycle_deduper.already_seen(&keys(&["b", "c", "a"])));
        assert!(cycle_deduper.already_seen(&keys(&["c", "a", "b"])));
        assert!(!cycle_deduper.already_seen(&keys(&["b", "a", "c"])));
        assert!(cycle_deduper.already_seen(&keys(&["c", "b", "a"])));
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CycleDeduperTest.java:44 CycleDeduperTest.badCycle_empty
    #[test]
    #[should_panic]
    fn bad_cycle_empty() {
        let mut cycle_deduper = CycleDeduper::new();
        cycle_deduper.already_seen(&[]);
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/CycleDeduperTest.java:49 CycleDeduperTest.badCycle_nonUniqueMembers
    #[test]
    #[should_panic(expected = "cycle doesn't have unique members")]
    fn bad_cycle_non_unique_members() {
        let mut cycle_deduper = CycleDeduper::new();
        cycle_deduper.already_seen(&keys(&["a", "b", "a"]));
    }

    fn print(cycle: &[SkyKey]) -> String {
        let mut message = String::from("cycle in dependency graph:");
        print_cycle(cycle, &mut message, &|k| format!("{:?}", k), &|_| false);
        message
    }

    /// The shapes `AbstractLabelCycleReporter.printCycle` draws (the Java at
    /// `AbstractLabelCycleReporter.java:146-172`): `.->` for the first key, `|` for the others,
    /// `` `-- `` followed by the first key again; a one-key cycle is a `[self-edge]`.
    #[test]
    fn print_cycle_draws_the_java_shapes() {
        assert_eq!(
            print(&keys(&["a", "b", "c"])),
            "cycle in dependency graph:\n.-> TestKey(\"a\")\n|   TestKey(\"b\")\n|   \
             TestKey(\"c\")\n`-- TestKey(\"a\")"
        );
        assert_eq!(
            print(&keys(&["a"])),
            "cycle in dependency graph:\n.-> TestKey(\"a\") [self-edge]\n`--"
        );
    }
}
