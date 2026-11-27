//! Port of `RewindingTest` (src/test/java/com/google/devtools/build/lib/skyframe/rewinding/
//! RewindingTest.java, with its `RewindingTestsHelper`): every case is a
//! `BuildIntegrationTestCase` (a full in-process build whose spawns a test controls to lose
//! inputs), which Blaze has no harness for.
//!
//! Covered meanwhile: `generatedHeaderRewound_lostInInputDiscovery` and
//! `ineffectiveRewindingResultsInLostInputTooManyTimes` in the `ActionExecutionFunction` fixture
//! (`action_execution_function.rs`, same names), the engine cases in blaze_skyframe
//! (`rewinding_evaluator_test.rs`), and the oracle probes of the task report.

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:155 RewindingTest.noLossSmokeTest
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn no_loss_smoke_test() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:160 RewindingTest.lostInputWithRewindingDisabled
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn lost_input_with_rewinding_disabled() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:165 RewindingTest.buildingParentFoundUndoneChildNotToleratedWithoutRewinding
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_parent_found_undone_child_not_tolerated_without_rewinding() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:170 RewindingTest.dependentActionsReevaluated
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn dependent_actions_reevaluated() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:175 RewindingTest.multipleLostInputsForRewindPlan
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn multiple_lost_inputs_for_rewind_plan() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:190 RewindingTest.ineffectiveRewindingResultsInLostInputTooManyTimes
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn ineffective_rewinding_results_in_lost_input_too_many_times() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:196 RewindingTest.interruptedDuringRewindStopsNormally
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn interrupted_during_rewind_stops_normally() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:202 RewindingTest.failureDuringRewindStopsNormally
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn failure_during_rewind_stops_normally() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:219 RewindingTest.intermediateActionRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn intermediate_action_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:224 RewindingTest.chainOfActionsRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn chain_of_actions_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:229 RewindingTest.nondeterministicActionRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn nondeterministic_action_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:234 RewindingTest.parallelTrackSharedActionsRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn parallel_track_shared_actions_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:239 RewindingTest.treeFileArtifactRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn tree_file_artifact_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:245 RewindingTest.treeArtifactRewound_allFilesLost
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn tree_artifact_rewound_all_files_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:251 RewindingTest.treeArtifactRewound_oneFileLost
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn tree_artifact_rewound_one_file_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:257 RewindingTest.actionTemplateExpansionRewound_notConcurrentWithTreeConsumers
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn action_template_expansion_rewound_not_concurrent_with_tree_consumers() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:262 RewindingTest.actionTemplateExpansionRewound_siblingActionsReExecuteConcurrently
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn action_template_expansion_rewound_sibling_actions_re_execute_concurrently() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:268 RewindingTest.generatedRunfilesRewound_allFilesLost
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_runfiles_rewound_all_files_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:273 RewindingTest.generatedRunfilesRewound_oneFileLost
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_runfiles_rewound_one_file_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:278 RewindingTest.dupeDirectAndRunfilesDependencyRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn dupe_direct_and_runfiles_dependency_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:283 RewindingTest.treeInRunfilesRewound
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn tree_in_runfiles_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:288 RewindingTest.inputsFromSameGeneratingActionSplitAmongNestedSetChildren
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn inputs_from_same_generating_action_split_among_nested_set_children() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:293 RewindingTest.generatedHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:299 RewindingTest.generatedHeaderRewound_lostInActionExecution
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:305 RewindingTest.generatedTransitiveHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_transitive_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:311 RewindingTest.generatedTransitiveHeaderRewound_lostInActionExecution
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn generated_transitive_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:317 RewindingTest.doneToDirtyDepForNodeInError
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn done_to_dirty_dep_for_node_in_error() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:322 RewindingTest.flakyActionFailsAfterRewind_raceWithIndirectConsumer_undoneDuringInputChecking
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn flaky_action_fails_after_rewind_race_with_indirect_consumer_undone_during_input_checking() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:328 RewindingTest.discoveredCppModuleLost
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn discovered_cpp_module_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:334 RewindingTest.multipleLostInputsWithSameDigest_rewoundTogether
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn multiple_lost_inputs_with_same_digest_rewound_together() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:339 RewindingTest.lostTopLevelOutputWithRewindingDisabled
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn lost_top_level_output_with_rewinding_disabled() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:344 RewindingTest.topLevelOutputRewound_regularFile
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:349 RewindingTest.topLevelOutputRewound_aspectOwned
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_aspect_owned() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:354 RewindingTest.topLevelOutputRewound_fileInTreeArtifact
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:359 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_regularFile
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_partially_built_target_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:364 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_fileInTreeArtifact
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_partially_built_target_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:369 RewindingTest.topLevelOutputRewound_ineffectiveRewinding
#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn top_level_output_rewound_ineffective_rewinding() {}
