//! Port of `RewindingTest` (src/test/java/com/google/devtools/build/lib/skyframe/rewinding/
//! RewindingTest.java, with its `RewindingTestsHelper`): every case is a
//! `BuildIntegrationTestCase` whose spawns the test controls (a `ControllableActionStrategyModule`
//! wrapping the `remote` and `standalone` strategies loses inputs on cue) and whose Skyframe graph
//! it watches (a `NotifyingHelper` listener collecting the keys marked dirty for rewinding).
//!
//! The in-process fixture is in the tree (blaze_buildtool_testing), but the cases still need two
//! seams Blaze does not have, both in other crates:
//!
//! 1. `BlazeModule.registerSpawnStrategies`: no module hook registers spawn strategies (blaze_exec
//!    `executor_setup::build_spawn_strategy_registry_with_policy` registers a fixed list of
//!    Bazel's modules, called from blaze_buildtool's `ExecutionTool`); with it, the helper's
//!    `ControllableActionStrategyModule` becomes a module passed to `new_with_modules` that wraps
//!    `registryBuilder.toStrategy(identifier)` (that method exists).
//! 2. `injectListenerAtStartOfNextBuild`: blaze_skyframe's `NotifyingHelper` listener
//!    (`graph_listener.rs`) is `cfg(test)` and crate-private, and no executor seam installs one for
//!    a command.
//!
//! Covered meanwhile: `generatedHeaderRewound_lostInInputDiscovery` and
//! `ineffectiveRewindingResultsInLostInputTooManyTimes` in the `ActionExecutionFunction` fixture
//! (`action_execution_function.rs`, same names), the engine cases in blaze_skyframe
//! (`rewinding_evaluator_test.rs`), and the end-to-end tests with a lost remote blob in
//! `crates/blaze/tests/rewind_lost_inputs_test.rs` (one input, and two copies of a shared action:
//! review RW-2).

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:155 RewindingTest.noLossSmokeTest
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn no_loss_smoke_test() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:160 RewindingTest.lostInputWithRewindingDisabled
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn lost_input_with_rewinding_disabled() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:165 RewindingTest.buildingParentFoundUndoneChildNotToleratedWithoutRewinding
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn building_parent_found_undone_child_not_tolerated_without_rewinding() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:170 RewindingTest.dependentActionsReevaluated
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn dependent_actions_reevaluated() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:175 RewindingTest.multipleLostInputsForRewindPlan
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn multiple_lost_inputs_for_rewind_plan() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:190 RewindingTest.ineffectiveRewindingResultsInLostInputTooManyTimes
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn ineffective_rewinding_results_in_lost_input_too_many_times() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:196 RewindingTest.interruptedDuringRewindStopsNormally
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn interrupted_during_rewind_stops_normally() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:202 RewindingTest.failureDuringRewindStopsNormally
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn failure_during_rewind_stops_normally() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:219 RewindingTest.intermediateActionRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn intermediate_action_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:224 RewindingTest.chainOfActionsRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn chain_of_actions_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:229 RewindingTest.nondeterministicActionRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn nondeterministic_action_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:234 RewindingTest.parallelTrackSharedActionsRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn parallel_track_shared_actions_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:239 RewindingTest.treeFileArtifactRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn tree_file_artifact_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:245 RewindingTest.treeArtifactRewound_allFilesLost
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn tree_artifact_rewound_all_files_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:251 RewindingTest.treeArtifactRewound_oneFileLost
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn tree_artifact_rewound_one_file_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:257 RewindingTest.actionTemplateExpansionRewound_notConcurrentWithTreeConsumers
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn action_template_expansion_rewound_not_concurrent_with_tree_consumers() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:262 RewindingTest.actionTemplateExpansionRewound_siblingActionsReExecuteConcurrently
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn action_template_expansion_rewound_sibling_actions_re_execute_concurrently() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:268 RewindingTest.generatedRunfilesRewound_allFilesLost
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_runfiles_rewound_all_files_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:273 RewindingTest.generatedRunfilesRewound_oneFileLost
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_runfiles_rewound_one_file_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:278 RewindingTest.dupeDirectAndRunfilesDependencyRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn dupe_direct_and_runfiles_dependency_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:283 RewindingTest.treeInRunfilesRewound
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn tree_in_runfiles_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:288 RewindingTest.inputsFromSameGeneratingActionSplitAmongNestedSetChildren
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn inputs_from_same_generating_action_split_among_nested_set_children() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:293 RewindingTest.generatedHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:299 RewindingTest.generatedHeaderRewound_lostInActionExecution
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:305 RewindingTest.generatedTransitiveHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_transitive_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:311 RewindingTest.generatedTransitiveHeaderRewound_lostInActionExecution
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn generated_transitive_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:317 RewindingTest.doneToDirtyDepForNodeInError
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn done_to_dirty_dep_for_node_in_error() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:322 RewindingTest.flakyActionFailsAfterRewind_raceWithIndirectConsumer_undoneDuringInputChecking
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn flaky_action_fails_after_rewind_race_with_indirect_consumer_undone_during_input_checking() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:328 RewindingTest.discoveredCppModuleLost
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn discovered_cpp_module_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:334 RewindingTest.multipleLostInputsWithSameDigest_rewoundTogether
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn multiple_lost_inputs_with_same_digest_rewound_together() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:339 RewindingTest.lostTopLevelOutputWithRewindingDisabled
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn lost_top_level_output_with_rewinding_disabled() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:344 RewindingTest.topLevelOutputRewound_regularFile
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:349 RewindingTest.topLevelOutputRewound_aspectOwned
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_aspect_owned() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:354 RewindingTest.topLevelOutputRewound_fileInTreeArtifact
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:359 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_regularFile
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_partially_built_target_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:364 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_fileInTreeArtifact
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_partially_built_target_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:369 RewindingTest.topLevelOutputRewound_ineffectiveRewinding
#[test]
#[ignore = "needs BlazeModule.registerSpawnStrategies and a NotifyingHelper listener seam (module docs)"]
fn top_level_output_rewound_ineffective_rewinding() {}
