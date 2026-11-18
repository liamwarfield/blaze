//! Skyframe keys of the execution phase.
//!
//! PORT: in Java `ActionLookupData`, `Artifact`, `ArtifactNestedSetKey`, `TargetCompletionKey`,
//! `TestCompletionKey` and `BuildDriverKey` are `SkyKey`s themselves. `ActionLookupData` and
//! `Artifact` belong to `blaze_actions`, which does not depend on `blaze_skyframe`, so they are
//! wrapped in newtypes here; their `Debug` is Java's `toString`.

use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact};
use blaze_collect::NestedSet;
use blaze_skyframe::{Key, SkyFunctionName, SkyKey};

use crate::action_execution_value::ActionExecutionValue;
use crate::artifact_function::ArtifactValue;
use crate::build_driver_function::BuildDriverValue;
use crate::completion_function::TargetCompletionValue;
use crate::test_completion_function::TestCompletionValue;

/// `SkyFunctions.ACTION_EXECUTION`: an `ActionLookupData` as a key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ActionExecutionKey(pub ActionLookupData);

impl fmt::Debug for ActionExecutionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Key for ActionExecutionKey {
    type Value = ActionExecutionValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("ACTION_EXECUTION");
}

/// `SkyFunctions.ARTIFACT`: a source artifact or a tree artifact as a key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ArtifactKey(pub Artifact);

impl fmt::Debug for ArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Key for ArtifactKey {
    type Value = ArtifactValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("ARTIFACT");
}

/// Returns the key for an artifact's value: the artifact itself for source and tree artifacts,
/// otherwise the key of its generating action.
// Java: com/google/devtools/build/lib/actions/Artifact.java:171 Artifact.key
pub fn artifact_key(artifact: &Artifact) -> SkyKey {
    if artifact.is_tree_artifact() || !artifact.has_known_generating_action() {
        return SkyKey::new(ArtifactKey(artifact.clone()));
    }
    SkyKey::new(ActionExecutionKey(
        artifact.get_generating_action_key().clone(),
    ))
}

/// `SkyKey` for a `NestedSet<Artifact>`.
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetKey.java:39 ArtifactNestedSetKey
#[derive(Clone)]
pub struct ArtifactNestedSetKey(pub NestedSet<Artifact>);

impl ArtifactNestedSetKey {
    /// Creates a key for a non-leaf, non-empty nested set.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetKey.java:56 ArtifactNestedSetKey.create
    pub fn create(set: NestedSet<Artifact>) -> ArtifactNestedSetKey {
        ArtifactNestedSetKey(set)
    }

    /// Returns the set of artifacts that this key represents: its direct leaves, then the keys of
    /// its non-leaf children (`getDirectDepKeys`).
    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetKey.java:99 ArtifactNestedSetKey.getDirectDepKeys
    pub fn get_direct_dep_keys(&self) -> Vec<SkyKey> {
        let mut keys = Vec::new();
        for leaf in self.0.get_leaves() {
            keys.push(artifact_key(&leaf));
        }
        for non_leaf in self.0.get_non_leaves() {
            keys.push(SkyKey::new(ArtifactNestedSetKey::create(non_leaf)));
        }
        keys
    }
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetKey.java:126 ArtifactNestedSetKey.equals
impl PartialEq for ArtifactNestedSetKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.shallow_equals(Some(&other.0))
    }
}

impl Eq for ArtifactNestedSetKey {}

impl Hash for ArtifactNestedSetKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.shallow_hash_code().hash(state);
    }
}

impl fmt::Debug for ArtifactNestedSetKey {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetKey.java:140 ArtifactNestedSetKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArtifactNestedSetKey[{}]", self.0)
    }
}

/// The (empty) value of an `ArtifactNestedSetKey`: the artifacts' values are read from their own
/// keys.
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetValue.java:22 ArtifactNestedSetValue
#[derive(Clone, Debug, Default)]
pub struct ArtifactNestedSetValue;

impl Key for ArtifactNestedSetKey {
    type Value = ArtifactNestedSetValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("ARTIFACT_NESTED_SET");
}

/// `TargetCompletionKey`: a top-level configured target whose requested output groups must be
/// built.
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:61 TargetCompletionValue.TargetCompletionKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TargetCompletionKey {
    /// `actionLookupKey()` (the `ConfiguredTargetKey`).
    pub action_lookup_key: ActionLookupKeyRef,
    /// `topLevelArtifactContext().outputGroups()` (the requested output groups, in order).
    pub output_groups: Vec<String>,
    /// `willTest()`.
    pub will_test: bool,
}

impl fmt::Debug for TargetCompletionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TargetCompletionKey{{actionLookupKey={}, topLevelArtifactContext=TopLevelArtifactContext{{outputGroups={:?}}}, willTest={}}}",
            self.action_lookup_key.0, self.output_groups, self.will_test
        )
    }
}

impl Key for TargetCompletionKey {
    type Value = TargetCompletionValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TARGET_COMPLETION");
}

/// `TestCompletionKey`: all runs and shards of a test target.
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:45 TestCompletionValue.TestCompletionKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TestCompletionKey {
    /// `configuredTargetKey()`.
    pub configured_target_key: ActionLookupKeyRef,
    /// `topLevelArtifactContext().outputGroups()`.
    pub output_groups: Vec<String>,
    /// `exclusiveTesting()`.
    pub exclusive_testing: bool,
}

impl fmt::Debug for TestCompletionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TestCompletionKey{{configuredTargetKey={}, exclusiveTesting={}}}",
            self.configured_target_key.0, self.exclusive_testing
        )
    }
}

impl Key for TestCompletionKey {
    type Value = TestCompletionValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TEST_COMPLETION");
}

/// `TestType` of a `BuildDriverKey`.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:33 BuildDriverKey.TestType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum TestType {
    NOT_TEST,
    PARALLEL,
    EXCLUSIVE,
    EXCLUSIVE_IF_LOCAL,
}

impl TestType {
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:40 TestType.isTest
    pub fn is_test(self) -> bool {
        self != TestType::NOT_TEST
    }
}

/// Wraps an `ActionLookupKey` (configured target or aspect) to drive its analysis and execution.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:26 BuildDriverKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BuildDriverKey {
    /// `getActionLookupKey()`.
    pub action_lookup_key: ActionLookupKeyRef,
    /// `getTopLevelArtifactContext().outputGroups()`.
    pub output_groups: Vec<String>,
    /// `isExplicitlyRequested()`.
    pub explicitly_requested: bool,
    /// `shouldSkipIncompatibleExplicitTargets()`.
    pub skip_incompatible_explicit_targets: bool,
    /// `isExtraActionTopLevelOnly()`.
    pub extra_action_top_level_only: bool,
    /// `getTestType()`.
    pub test_type: TestType,
}

impl fmt::Debug for BuildDriverKey {
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:164 BuildDriverKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildDriverKey of ActionLookupKey: {}",
            self.action_lookup_key.0
        )
    }
}

impl Key for BuildDriverKey {
    type Value = BuildDriverValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_DRIVER");
}
