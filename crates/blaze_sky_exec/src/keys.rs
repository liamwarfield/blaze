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
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName, SkyKey};

use blaze_exec::actions::FileArtifactValue;

use crate::action_execution_value::ActionExecutionValue;
use crate::artifact_function::ArtifactValue;
use crate::build_driver_function::BuildDriverValue;
use crate::completion_function::{AspectCompletionValue, TargetCompletionValue};
use crate::test_completion_function::TestCompletionValue;

/// `FileArtifactValue.equals` for change pruning: the enum's `==` (each Java subclass's
/// `equals`), except that a value that is or wraps a remote file is never equal. PORT: Java's
/// remote `equals` checks the class one way only (a value with materialization data equals a
/// plain one, not the reverse); the merged variant compares symmetrically and could prune where
/// Java does not, so it does not prune at all.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:756 RemoteFileArtifactValue.equals
fn file_values_equal_for_pruning(old: &FileArtifactValue, new: &FileArtifactValue) -> bool {
    fn is_remote(value: &FileArtifactValue) -> bool {
        match value {
            FileArtifactValue::RemoteFile { .. } => true,
            FileArtifactValue::ResolvedSymlink { delegate, .. }
            | FileArtifactValue::Proxy { delegate, .. } => is_remote(delegate),
            _ => false,
        }
    }
    !is_remote(old) && !is_remote(new) && old == new
}

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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:121 SkyFunctions.ACTION_EXECUTION (createSemiHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::SemiHermetic;
    /// Java: the file values (a map, `FileArtifactValue.equals`), the tree artifact values
    /// (`TreeArtifactValue.equals`), the rich artifact data (`RunfilesArtifactValue.equals`) and the
    /// discovered modules (`shallowEquals`). PORT: `TreeArtifactValue` and
    /// `RunfilesArtifactValue` have no Java-faithful equality yet, so a value with tree artifacts
    /// or rich artifact data is never equal; the discovered modules are compared with
    /// `shallowEquals` (the same set object, or both empty). Remote file values are never equal (see
    /// [`file_values_equal_for_pruning`]). Rust prunes less than Java, never more.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:232 ActionExecutionValue.equals
    fn values_equal(old: &ActionExecutionValue, new: &ActionExecutionValue) -> bool {
        let (old_files, new_files) = (old.get_all_file_values(), new.get_all_file_values());
        old.get_all_tree_artifact_values().is_empty()
            && new.get_all_tree_artifact_values().is_empty()
            && old.get_rich_artifact_data().is_none()
            && new.get_rich_artifact_data().is_none()
            && old
                .get_discovered_modules()
                .shallow_equals(Some(&new.get_discovered_modules()))
            && old_files.len() == new_files.len()
            && old_files.iter().all(|(artifact, value)| {
                new_files
                    .get(artifact)
                    .is_some_and(|other| file_values_equal_for_pruning(value, other))
            })
    }
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
    // Java: com/google/devtools/build/lib/actions/Artifact.java:159 Artifact.ARTIFACT (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: `ArtifactFunction` returns a `FileArtifactValue` (each subclass's `equals`), a
    /// `TreeArtifactValue` (`TreeArtifactValue.equals`) or a `MissingArtifactValue` (no `equals`:
    /// identity). PORT: tree values are never equal (no Java-faithful `TreeArtifactValue`
    /// equality yet), nor are remote file values (see [`file_values_equal_for_pruning`]): Rust
    /// prunes less than Java, never more.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:131 ArtifactFunction.compute
    fn values_equal(old: &ArtifactValue, new: &ArtifactValue) -> bool {
        match (old, new) {
            (ArtifactValue::File(old), ArtifactValue::File(new)) => {
                file_values_equal_for_pruning(old, new)
            }
            _ => false,
        }
    }
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

/// Represent a "promise" that the Artifacts under a NestedSet are evaluated by Skyframe (the
/// artifacts' values are read from their own keys).
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetValue.java:27 ArtifactNestedSetValue
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ArtifactNestedSetValue {
    /// All artifacts in this nested set are present.
    ALL_PRESENT,
    /// Some artifacts in this nested set are missing.
    SOME_MISSING,
}

impl Key for ArtifactNestedSetKey {
    type Value = ArtifactNestedSetValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("ARTIFACT_NESTED_SET");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:123 SkyFunctions.ARTIFACT_NESTED_SET (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): Java's `ArtifactNestedSetValue` is a
    // `NotComparableSkyValue` "to prohibit value-based change pruning". Its `PartialEq` must not
    // be used for pruning.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetValue.java:27 ArtifactNestedSetValue
}

/// `TargetCompletionKey`: a top-level configured target whose requested output groups must be
/// built.
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:61 TargetCompletionValue.TargetCompletionKey
#[derive(Clone, PartialEq, Eq, Hash)]
// (`create` is the struct literal; `functionName` is `Key::FUNCTION`; `key`/`keys` are built by
// the callers.)
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:55 TargetCompletionValue.TargetCompletionKey.create
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:66 TargetCompletionValue.TargetCompletionKey.functionName
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:33 TargetCompletionValue.key
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:40 TargetCompletionValue.keys
// PORT: skipped, shareability only matters to remote analysis caching (unported!): TargetCompletionValue.TargetCompletionKey.valueIsShareable
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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:106 SkyFunctions.TARGET_COMPLETION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns one singleton, equal to itself.
    // Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:29 TargetCompletionValue.INSTANCE
    fn values_equal(_old: &TargetCompletionValue, _new: &TargetCompletionValue) -> bool {
        true
    }
}

/// `AspectCompletionKey`: a top-level aspect whose requested output groups must be built.
// Java: com/google/devtools/build/lib/skyframe/AspectCompletionValue.java:41 AspectCompletionValue.AspectCompletionKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AspectCompletionKey {
    /// `topLevelArtifactContext().outputGroups()` (the requested output groups, in order).
    pub output_groups: Vec<String>,
    /// `actionLookupKey()` (the `AspectKey`).
    pub action_lookup_key: ActionLookupKeyRef,
}

impl AspectCompletionKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectCompletionValue.java:43 AspectCompletionKey.create
    pub fn create(
        aspect_key: ActionLookupKeyRef,
        output_groups: Vec<String>,
    ) -> AspectCompletionKey {
        AspectCompletionKey {
            output_groups,
            action_lookup_key: aspect_key,
        }
    }
}

impl fmt::Debug for AspectCompletionKey {
    // AutoValue toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AspectCompletionKey{{topLevelArtifactContext=TopLevelArtifactContext{{outputGroups={:?}}}, actionLookupKey={}}}",
            self.output_groups, self.action_lookup_key.0
        )
    }
}

impl Key for AspectCompletionKey {
    type Value = AspectCompletionValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("ASPECT_COMPLETION");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:108 SkyFunctions.ASPECT_COMPLETION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns one singleton, equal to itself.
    // Java: com/google/devtools/build/lib/skyframe/AspectCompletionValue.java:31 AspectCompletionValue.INSTANCE
    fn values_equal(_old: &AspectCompletionValue, _new: &AspectCompletionValue) -> bool {
        true
    }
}

/// `TestCompletionKey`: all runs and shards of a test target.
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:45 TestCompletionValue.TestCompletionKey
#[derive(Clone, PartialEq, Eq, Hash)]
// (`create` is the struct literal; `functionName` is `Key::FUNCTION`; `key`/`keys` are built by
// the callers.)
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:62 TestCompletionValue.TestCompletionKey.create
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:78 TestCompletionValue.TestCompletionKey.functionName
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:36 TestCompletionValue.key
// Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:43 TestCompletionValue.keys
// PORT: skipped, shareability (remote analysis caching, unported!) and key interning: TestCompletionValue.TestCompletionKey.valueIsShareable TestCompletionValue.TestCompletionKey.getSkyKeyInterner
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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:109 SkyFunctions.TEST_COMPLETION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns one singleton, equal to itself.
    // Java: com/google/devtools/build/lib/skyframe/TestCompletionValue.java:32 TestCompletionValue.TEST_COMPLETION_MARKER
    fn values_equal(_old: &TestCompletionValue, _new: &TestCompletionValue) -> bool {
        true
    }
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
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:93 BuildDriverKey.getActionLookupKey
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:97 BuildDriverKey.isExplicitlyRequested
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:101 BuildDriverKey.shouldSkipIncompatibleExplicitTargets
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:109 BuildDriverKey.isExtraActionTopLevelOnly
// (these getters are the public fields)
#[derive(Clone)]
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
    ///
    /// PORT: Java's key has no test type: `BuildDriverFunction` asks the build's
    /// `TestTypeResolver` once the target is analyzed (`AnalysisSeam::determine_test_type`).
    /// The path without merged analysis and execution, which evaluates these keys after its
    /// analysis (blaze_runtime), resolves it when it creates the keys; with Skymeld the field is
    /// `NOT_TEST` and unused.
    pub test_type: TestType,
    /// `keepGoing()`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:113 BuildDriverKey.keepGoing
    pub keep_going: bool,
}

/// `SkyFunctions.TOP_LEVEL_ASPECTS`: the function name of a `TopLevelAspectsKey`.
pub const TOP_LEVEL_ASPECTS: &str = "TOP_LEVEL_ASPECTS";

impl BuildDriverKey {
    /// A key for a `ConfiguredTargetKey` of a build with merged analysis and execution: the test
    /// type is resolved by `BuildDriverFunction` once the target is analyzed (the
    /// `TestTypeResolver`), so the key carries `NOT_TEST`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:72 BuildDriverKey.ofConfiguredTarget
    pub fn of_configured_target(
        action_lookup_key: ActionLookupKeyRef,
        output_groups: Vec<String>,
        explicitly_requested: bool,
        skip_incompatible_explicit_targets: bool,
        extra_action_top_level_only: bool,
        keep_going: bool,
    ) -> BuildDriverKey {
        BuildDriverKey {
            action_lookup_key,
            output_groups,
            explicitly_requested,
            skip_incompatible_explicit_targets,
            extra_action_top_level_only,
            test_type: TestType::NOT_TEST,
            keep_going,
        }
    }

    /// A key for a `TopLevelAspectsKey`.
    ///
    /// PORT: Java's `isTopLevelAspectDriver` field is not carried (adding a field would change
    /// every struct literal of this key); [`BuildDriverKey::is_top_level_aspect_driver`] derives
    /// it from the kind of the action lookup key, which Java's two factories pair the same way.
    /// The test type of an aspect driver is `NOT_TEST` (Java: `state.testType = NOT_TEST` for a
    /// `TopLevelAspectsValue`).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:55 BuildDriverKey.ofTopLevelAspect
    pub fn of_top_level_aspect(
        action_lookup_key: ActionLookupKeyRef,
        output_groups: Vec<String>,
        explicitly_requested: bool,
        skip_incompatible_explicit_targets: bool,
        extra_action_top_level_only: bool,
        keep_going: bool,
    ) -> BuildDriverKey {
        blaze_util::check_argument!(
            action_lookup_key.function_name() == TOP_LEVEL_ASPECTS,
            "{} is not a TopLevelAspectsKey",
            action_lookup_key
        );
        BuildDriverKey {
            action_lookup_key,
            output_groups,
            explicitly_requested,
            skip_incompatible_explicit_targets,
            extra_action_top_level_only,
            test_type: TestType::NOT_TEST,
            keep_going,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:103 BuildDriverKey.isTopLevelAspectDriver
    pub fn is_top_level_aspect_driver(&self) -> bool {
        self.action_lookup_key.function_name() == TOP_LEVEL_ASPECTS
    }
}

/// Equality over the action lookup key, the top-level artifact context and
/// `explicitlyRequested` (not `keepGoing` or the other fields).
// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:123 BuildDriverKey.equals
impl PartialEq for BuildDriverKey {
    fn eq(&self, other: &Self) -> bool {
        self.action_lookup_key == other.action_lookup_key
            && self.output_groups == other.output_groups
            && self.explicitly_requested == other.explicitly_requested
    }
}

impl Eq for BuildDriverKey {}

// Java: com/google/devtools/build/lib/skyframe/BuildDriverKey.java:133 BuildDriverKey.hashCode
impl std::hash::Hash for BuildDriverKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.action_lookup_key.hash(state);
        self.output_groups.hash(state);
        self.explicitly_requested.hash(state);
    }
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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:155 SkyFunctions.BUILD_DRIVER (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    // Default `values_equal` (never equal): Java's `BuildDriverValue` has no `equals`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverValue.java:19 BuildDriverValue
}
