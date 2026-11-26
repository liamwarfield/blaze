//! Ports of `skyframe/CoverageReportValue.java` (with its `CoverageReportKey`) and of the
//! `Precomputed` of `skyframe/CoverageReportFunction.java`.
//!
//! PORT: Java's classes live in `lib/skyframe`; they are here, next to `WorkspaceStatusKey`, because
//! `CoverageReportActionFactory.CoverageReportActionsWrapper` (lib/analysis/test) names
//! `CoverageReportValue.COVERAGE_REPORT_KEY`. `CoverageReportFunction` itself is registered by
//! blaze_sky_top's executor ([`compute_coverage_report_value`]).

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::actions::action_lookup::{ActionLookupKey, ActionLookupKeyRef};
use blaze_actions::{ActionKeyContext, ActionRef};
use blaze_cmdline::Label;
use blaze_skyframe::precomputed::Precomputed;

/// `SkyFunctions.COVERAGE_REPORT`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:132 SkyFunctions.COVERAGE_REPORT
pub const COVERAGE_REPORT_FUNCTION_NAME: &str = "COVERAGE_REPORT";

/// The key of the coverage report value. There should only ever be one `CoverageReportValue`
/// in the graph.
// Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:41 CoverageReportValue.CoverageReportKey
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoverageReportKey;

/// `CoverageReportValue.COVERAGE_REPORT_KEY`.
// Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:33 CoverageReportValue.COVERAGE_REPORT_KEY
pub const COVERAGE_REPORT_KEY: CoverageReportKey = CoverageReportKey;

impl CoverageReportKey {
    /// The key as an `ActionLookupKey` (the owner of the coverage report artifacts).
    pub fn action_lookup_key() -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(COVERAGE_REPORT_KEY)
    }
}

impl fmt::Display for CoverageReportKey {
    // Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:62 CoverageReportKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CoverageReportKeySingleton")
    }
}

impl fmt::Debug for CoverageReportKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ActionLookupKey for CoverageReportKey {
    // Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:56 CoverageReportKey.getLabel
    fn get_label(&self) -> Option<Label> {
        None
    }

    // Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:50 CoverageReportKey.getConfigurationKey
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }

    // Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:45 CoverageReportKey.functionName
    fn function_name(&self) -> &'static str {
        COVERAGE_REPORT_FUNCTION_NAME
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<CoverageReportKey>()
    }

    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        COVERAGE_REPORT_FUNCTION_NAME.hash(&mut h);
        h.finish()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A SkyValue to store the coverage report Action and Artifacts.
// Java: com/google/devtools/build/lib/skyframe/CoverageReportValue.java:29 CoverageReportValue
#[derive(Debug)]
pub struct CoverageReportValue {
    /// `BasicActionLookupValue.actions`.
    pub actions: Vec<ActionRef>,
}

impl blaze_skyframe::Key for CoverageReportKey {
    type Value = CoverageReportValue;
    const FUNCTION: blaze_skyframe::SkyFunctionName =
        blaze_skyframe::SkyFunctionName(COVERAGE_REPORT_FUNCTION_NAME);
    const HERMETICITY: blaze_skyframe::FunctionHermeticity =
        blaze_skyframe::FunctionHermeticity::Hermetic;
}

/// The coverage report actions injected before the evaluation of the coverage artifacts
/// (Java's `ImmutableList<ActionAnalysisMetadata>`; equal when they are the same actions).
#[derive(Debug, Clone)]
pub struct CoverageReportActions(pub Vec<ActionRef>);

impl PartialEq for CoverageReportActions {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    }
}

// Java: com/google/devtools/build/lib/skyframe/CoverageReportFunction.java:33 CoverageReportFunction.COVERAGE_REPORT_KEY
pub static COVERAGE_REPORT_ACTIONS: Precomputed<CoverageReportActions> =
    Precomputed::new("coverage_report_actions");

/// `CoverageReportFunction.compute`.
// Java: com/google/devtools/build/lib/skyframe/CoverageReportFunction.java:43 CoverageReportFunction.compute
pub fn compute_coverage_report_value(
    action_key_context: &ActionKeyContext,
    env: &mut blaze_skyframe::Environment<'_>,
) -> Result<Option<CoverageReportValue>, blaze_skyframe::ComputeError> {
    let Some(actions) = COVERAGE_REPORT_ACTIONS.get(env)? else {
        return Ok(None);
    };
    if let Err(e) =
        crate::rule::actions_util::assign_owners_and_throw_if_conflict_tolerating_shared_actions(
            action_key_context,
            &actions.0,
            &CoverageReportKey::action_lookup_key(),
        )
    {
        panic!(
            "IllegalStateException: Issues not expected in coverage: {COVERAGE_REPORT_KEY}: {e:?}"
        );
    }
    Ok(Some(CoverageReportValue {
        actions: actions.0.clone(),
    }))
}
