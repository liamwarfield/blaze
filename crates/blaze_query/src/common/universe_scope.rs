//! Port of `query2/common/UniverseScope.java`.

use blaze_query_engine::QueryExpression;
use blaze_sky_loading::prepare_deps::TargetPatternSequence;
use blaze_vfs::PathFragment;

/// The universe a SkyQuery evaluates in: a constant list of target patterns, or the target
/// patterns of the query expression.
// Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:27 UniverseScope
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UniverseScope {
    /// Java `ConstantUniverseScope`.
    Constant(Vec<String>),
    /// Java `InferredUniverseScope` (`INFER_FROM_QUERY_EXPRESSION`).
    InferFromQueryExpression,
}

impl UniverseScope {
    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:31 UniverseScope.EMPTY
    pub fn empty() -> UniverseScope {
        UniverseScope::Constant(Vec::new())
    }

    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:34 UniverseScope.fromUniverseScopeList
    pub fn from_universe_scope_list(constant_universe_scope: Vec<String>) -> UniverseScope {
        UniverseScope::Constant(constant_universe_scope)
    }

    /// The key of the universe (Java `UniverseSkyKey`, a `TargetPatternSequence`).
    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:63 UniverseScope.ConstantUniverseScope.getUniverseKey
    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:83 UniverseScope.InferredUniverseScope.getUniverseKey
    pub fn get_universe_key(
        &self,
        expr: Option<&QueryExpression>,
        offset: &PathFragment,
    ) -> TargetPatternSequence {
        match self {
            UniverseScope::Constant(constant_universe_scope) => {
                TargetPatternSequence::key(constant_universe_scope.clone(), offset.clone())
            }
            UniverseScope::InferFromQueryExpression => {
                let mut target_patterns: indexmap::IndexSet<String> = indexmap::IndexSet::new();
                expr.expect("checkNotNull(expr)")
                    .collect_target_patterns(&mut |p| {
                        target_patterns.insert(p);
                    });
                TargetPatternSequence::key(target_patterns.into_iter().collect(), offset.clone())
            }
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:53 UniverseScope.ConstantUniverseScope.isEmpty
    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:73 UniverseScope.InferredUniverseScope.isEmpty
    pub fn is_empty(&self) -> bool {
        match self {
            UniverseScope::Constant(c) => c.is_empty(),
            UniverseScope::InferFromQueryExpression => false,
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:58 UniverseScope.ConstantUniverseScope.getConstantValueMaybe
    // Java: com/google/devtools/build/lib/query2/common/UniverseScope.java:78 UniverseScope.InferredUniverseScope.getConstantValueMaybe
    pub fn get_constant_value_maybe(&self) -> Option<&[String]> {
        match self {
            UniverseScope::Constant(c) => Some(c),
            UniverseScope::InferFromQueryExpression => None,
        }
    }
}
