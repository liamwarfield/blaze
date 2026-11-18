//! Port of `CycleInfo`.

use crate::SkyKey;
use blaze_util::{check_argument, check_state};
use std::collections::HashSet;
use std::fmt;

/// Data for a single cycle in the graph, together with the path to the cycle. For any value, the
/// head of path to the cycle should be the value itself, or, if the value is actually in the
/// cycle, the cycle should start with the value.
///
/// PORT: Java's abstract class with the `CycleInfoWithDetails` and `CycleInfoNoDetails`
/// subclasses is a closed hierarchy, so it is an enum.
// Java: com/google/devtools/build/skyframe/CycleInfo.java:32 CycleInfo
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CycleInfo {
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:133 CycleInfoWithDetails
    WithDetails {
        path_to_cycle: Vec<SkyKey>,
        cycle: Vec<SkyKey>,
    },
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:111 CycleInfoNoDetails
    NoDetails,
}

impl CycleInfo {
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:41 CycleInfo.createCycleInfo
    pub fn create_cycle_info_without_path(cycle: Vec<SkyKey>) -> CycleInfo {
        Self::with_details(vec![], cycle)
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:46 CycleInfo.createCycleInfo
    pub fn create_cycle_info(path_to_cycle: Vec<SkyKey>, cycle: Vec<SkyKey>) -> CycleInfo {
        Self::with_details(path_to_cycle, cycle)
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:52 CycleInfo.cycleInfoNoDetails
    pub fn cycle_info_no_details() -> CycleInfo {
        CycleInfo::NoDetails
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:137 CycleInfoWithDetails.<init>
    fn with_details(path_to_cycle: Vec<SkyKey>, cycle: Vec<SkyKey>) -> CycleInfo {
        let info = CycleInfo::WithDetails {
            path_to_cycle,
            cycle,
        };
        if let CycleInfo::WithDetails { cycle, .. } = &info {
            check_argument!(!cycle.is_empty(), "Cycle cannot be empty: {:?}", info);
        }
        info
    }

    /// If a cycle is already known, but we are processing a value in the middle of the cycle, we
    /// need to shift the cycle so that the value is at the head.
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:145 CycleInfoWithDetails.<init>
    fn rotated(cycle: &[SkyKey], cycle_start: usize) -> CycleInfo {
        check_state!(
            cycle_start < cycle.len(),
            "{} >= {} ??",
            cycle_start,
            cycle.len()
        );
        let mut rotated = cycle[cycle_start..].to_vec();
        rotated.extend_from_slice(&cycle[..cycle_start]);
        CycleInfo::WithDetails {
            path_to_cycle: vec![],
            cycle: rotated,
        }
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:33 CycleInfo.getCycle
    pub fn get_cycle(&self) -> &[SkyKey] {
        match self {
            CycleInfo::WithDetails { cycle, .. } => cycle,
            CycleInfo::NoDetails => panic!("unexpected access of cycle details"),
        }
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:35 CycleInfo.getPathToCycle
    pub fn get_path_to_cycle(&self) -> &[SkyKey] {
        match self {
            CycleInfo::WithDetails { path_to_cycle, .. } => path_to_cycle,
            CycleInfo::NoDetails => panic!("unexpected access of cycle details"),
        }
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:37 CycleInfo.getTopKey
    pub fn get_top_key(&self) -> &SkyKey {
        match self {
            CycleInfo::WithDetails {
                path_to_cycle,
                cycle,
            } => path_to_cycle.first().unwrap_or(&cycle[0]),
            CycleInfo::NoDetails => panic!("unexpected access of cycle details"),
        }
    }

    // Java: com/google/devtools/build/skyframe/CycleInfo.java:39 CycleInfo.hasCycleDetails
    pub fn has_cycle_details(&self) -> bool {
        matches!(self, CycleInfo::WithDetails { .. })
    }

    /// Given a cycle and a value, if the value is part of the cycle, shift the cycle. Otherwise,
    /// prepend the value to the head of pathToCycle.
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:58 CycleInfo.normalizeCycle
    fn normalize_cycle(value: &SkyKey, cycle: &CycleInfo) -> Option<CycleInfo> {
        if let Some(index) = cycle.get_cycle().iter().position(|k| k == value) {
            if !cycle.get_path_to_cycle().is_empty() {
                // The head value we are considering is already part of a cycle, but we have
                // reached it by a roundabout way. Since we should have reached it directly as
                // well, filter this roundabout way out. Example (c has a dependence on top):
                //          top
                //         /  ^
                //        a   |
                //       / \ /
                //      b-> c
                // In the traversal, we start at top, visit a, then c, then top. This yields the
                // cycle {top,a,c}. Then we visit b, getting (b, {top,a,c}). Then we construct the
                // full error for a. The error should just be the cycle {top,a,c}, but we have an
                // extra copy of it via the path through b.
                return None;
            }
            return Some(CycleInfo::rotated(cycle.get_cycle(), index));
        }
        let mut path = Vec::with_capacity(cycle.get_path_to_cycle().len() + 1);
        path.push(value.clone());
        path.extend_from_slice(cycle.get_path_to_cycle());
        Some(CycleInfo::create_cycle_info(
            path,
            cycle.get_cycle().to_vec(),
        ))
    }

    /// Normalize multiple cycles. This includes removing multiple paths to the same cycle, so
    /// that a value does not depend on the same cycle multiple ways through the same child value.
    /// Note that a value can still depend on the same cycle multiple ways, it's just that each way
    /// must be through a different child value (a path with a different first element).
    ///
    /// If any of the given cycles are without details (created using
    /// [`CycleInfo::cycle_info_no_details`]) then a single `cycle_info_no_details` will be
    /// returned.
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:96 CycleInfo.prepareCycles
    pub fn prepare_cycles<'a>(
        value: &SkyKey,
        cycles: impl IntoIterator<Item = &'a CycleInfo>,
    ) -> Vec<CycleInfo> {
        let mut already_done_cycles: HashSet<Vec<SkyKey>> = HashSet::new();
        let mut result = vec![];
        for cycle in cycles {
            if !cycle.has_cycle_details() {
                return vec![cycle.clone()];
            }
            if let Some(normalized) = Self::normalize_cycle(value, cycle)
                && already_done_cycles.insert(normalized.get_cycle().to_vec())
            {
                result.push(normalized);
            }
        }
        result
    }
}

impl fmt::Display for CycleInfo {
    // Java: com/google/devtools/build/skyframe/CycleInfo.java:199 CycleInfoWithDetails.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CycleInfo::WithDetails {
                path_to_cycle,
                cycle,
            } => {
                write_iterable(f, path_to_cycle)?;
                f.write_str(" -> ")?;
                write_iterable(f, cycle)
            }
            // PORT: Java prints Object.toString (class name and identity hash).
            CycleInfo::NoDetails => f.write_str("CycleInfoNoDetails"),
        }
    }
}

impl fmt::Debug for CycleInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Guava `Iterables.toString`: `[a, b, c]`.
pub(crate) fn write_iterable(f: &mut fmt::Formatter<'_>, keys: &[SkyKey]) -> fmt::Result {
    f.write_str("[")?;
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{k:?}")?;
    }
    f.write_str("]")
}
