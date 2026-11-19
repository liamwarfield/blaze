# I1 phase 1: the node-entry layer (worker I1-A)

Tree: worktree `i1-a`, based on commit 48b61bb, uncommitted. Files under
`crates/blaze_skyframe/` only (plus the `Cargo.lock` lines for the two new dependencies). `J:` is
`src/main/java/com/google/devtools/build/skyframe/`, `JT:` the same under `src/test/java`. Line
numbers below are in the files as they are now.

Nothing here is used by the evaluator yet; every new module starts with
`#![allow(dead_code)] // I1 phase 1: wired into the evaluator in phase 2`.

## (a) Java classes

| Java class | Java file:lines | Rust file | State | Tests |
| --- | --- | --- | --- | --- |
| `Version` (+ `MinimalVersion`, `ConstantVersion`) | `J:Version.java:23-103` | `src/version.rs` (`pub enum Version`) | ported | through the entry tests |
| `IntVersion` | `J:IntVersion.java:20-71` | `src/version.rs` (`pub struct IntVersion`) | ported (no interning) | through the entry tests |
| `NodeVersion` (+ `ChangePruned`) | `J:NodeVersion.java:22-55` | `src/version.rs` (`pub enum NodeVersion`) | ported | change-pruning tests |
| `NodeEntry` (+ `DependencyState`, `LifecycleState`, `DirtyType`, `MarkedDirtyResult`, `NodeValueAndRdepsToSignal`) | `J:NodeEntry.java:35-680` | `src/node_entry.rs` (trait `NodeEntry`, `pub enum DirtyType`) | ported; `resetEvaluationFromScratch`, `addExternalDep` documented as unported, `REWIND` fails | — |
| `InMemoryNodeEntry` | `J:InMemoryNodeEntry.java:24-73` | `src/in_memory_node_entry.rs` (`enum InMemoryNodeEntry`) | ported | — |
| `AbstractInMemoryNodeEntry` | `J:AbstractInMemoryNodeEntry.java:65-261` | `src/abstract_in_memory_node_entry.rs` (fields struct + trait) | ported; `addExternalDep` unported | `InMemoryNodeEntryTest` |
| `IncrementalInMemoryNodeEntry` (+ `IncrementalBuildingState`) | `J:IncrementalInMemoryNodeEntry.java:32-483` | `src/incremental_in_memory_node_entry.rs`; `IncrementalBuildingState` in `src/dirty_building_state.rs` | partly: rewinding, resets and Skyfocus unported | `InMemoryNodeEntryTest`, `IncrementalInMemoryNodeEntryTest` |
| `IncrementalInMemoryNodeEntry.ResetInitialBuildingState`, `ResetIncrementalBuildingState` | `J:IncrementalInMemoryNodeEntry.java:489-538` | — | not ported (rewinding) | ignored |
| `NonIncrementalInMemoryNodeEntry` (+ `NonIncrementalBuildingState`) | `J:NonIncrementalInMemoryNodeEntry.java:40-279` | `src/non_incremental_in_memory_node_entry.rs` | partly: rewinding, resets unported | `InMemoryNodeEntryTest` |
| `NonIncrementalInMemoryNodeEntry.RewoundNonIncrementalBuildingState` | `J:NonIncrementalInMemoryNodeEntry.java:286-303` | — | not ported (rewinding) | ignored |
| `DirtyBuildingState` | `J:DirtyBuildingState.java:38-306` | `src/dirty_building_state.rs` | ported (`addExternalDep` ported, its only caller is unported) | through the entry tests |
| `InitialBuildingState` | `J:InitialBuildingState.java:23-50` | `src/dirty_building_state.rs` (`DirtyBuildingStateKind::Initial`) | ported | through the entry tests |
| `ReverseDepsUtility` | `J:ReverseDepsUtility.java:59-412` | `src/reverse_deps_utility.rs` | ported (storage format simplified, see (b)) | `ReverseDepsUtilityTest` |
| `KeyToConsolidate` (+ `Op`) | `J:KeyToConsolidate.java:34-175` | `src/reverse_deps_utility.rs` | ported (no interning) | `ReverseDepsUtilityTest` |
| `ValueWithMetadata` (+ `ValueWithEvents`, `NotComparableValueWithEvents`, `ErrorInfoValue`) | `J:ValueWithMetadata.java:26-253` | `src/value_with_metadata.rs` | ported; `ErrorInfoValue.equals` unported | through the entry tests |
| `Reportable` as stored by a node | `lib/events/Reportable.java` | `src/value_with_metadata.rs` (`StoredReportable`, moved from `environment.rs`) | moved, `Clone + PartialEq + Eq + Hash` added | — |
| `GroupedDeps.appendGroups` / `appendNextGroup` | `J:GroupedDeps.java:149-179` | `src/grouped_deps.rs` | ported (`compress`/`decompress`/`WithHashSet` not: memory format / partial reevaluation) | `InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups*` (GroupedDepsTest has no case for it) |
| `SkyValue.equals` hook (decision D10) | `J:DirtyBuildingState.java:214` | `src/lib.rs` `Key::values_equal`, `DynKey::dyn_values_equal` | added | change-pruning tests |

## (b) `// PORT:` deviations introduced

| Where | Java | Deviation and why |
| --- | --- | --- |
| `version.rs:12` | `Version.java:23` | Interface with three implementations is a closed enum; singleton identity is variant equality. |
| `version.rs:100` | `Object.toString` | `MinimalVersion`/`ConstantVersion` print their class name (Java's identity hash is not reproducible). |
| `version.rs:111` | `IntVersion.java:22-26` | No interner; equality was by value anyway. |
| `version.rs:155` | `NodeVersion.java:22` | Interface implemented by `Version` and `ChangePruned` is a two-variant enum. |
| `value_with_metadata.rs:15` | `Reportable` | A `Postable` compares by identity (`Arc::ptr_eq`, pointer hash); a Java `Postable` overriding `equals` would compare by value. |
| `value_with_metadata.rs:72` | `ValueWithMetadata.java:29` | `NO_EVENTS` is a new empty set per call (`Order::empty_set`); all empty sets are `shallowEquals`. |
| `value_with_metadata.rs:81` | `ValueWithMetadata.java:46-58, 225-252` | Java's bare `SkyValue` (no error, no events) is the `Bare` variant of the one enum; the static helpers take that enum. |
| `value_with_metadata.rs:88`, `:151` | `ValueWithMetadata.java:84-91, 140-145` | No `NotComparableValueWithEvents` variant: comparability is the key's `values_equal`. |
| `value_with_metadata.rs:205` | `ValueWithMetadata.java:109-124, 182-201` | `equals` takes the node's key (value equality is the key's, D10); Java's `this == o` shortcut is dropped (Rust cannot see `NotComparableSkyValue`, and a stored value is never compared with itself); a bare value is never equal to a wrapped one. |
| `value_with_metadata.rs:281` | `toString`s | A `SkyValue` (`dyn Any`) prints as `<SkyValue>` in crash messages. |
| `node_entry.rs:111` | `NodeEntry.java:95-136` | `DirtyType::Rewind` kept; marking a node `Rewind` fails with `unported!`. |
| `node_entry.rs:135` | `NodeEntry.java:182-185` | `MarkedDirtyResult` holds a copy of the reverse deps (same content and order), not a live view. |
| `node_entry.rs:203` | `NodeEntry.java:35` | `synchronized` is the caller's lock (graph `Mutex`); no `InterruptedException`; methods take `&self`/`&mut self`. |
| `node_entry.rs:334`, `incremental_in_memory_node_entry.rs:163`, `non_incremental_in_memory_node_entry.rs:82` | `NodeEntry.java:341` | `max_transitive_source_version` accepted and ignored (both in-memory entries ignore it at the pin). |
| `node_entry.rs:511` | `NodeEntry.java:561` | `get_temporary_direct_deps` takes `&mut self` (lazy creation). |
| `node_entry.rs:537`, `dirty_building_state.rs:375` | `NodeEntry.java:627`, `DirtyBuildingState.java:270` | `getResetDirectDeps` is the base `ImmutableSet.of()`; reset states not ported. |
| `dirty_building_state.rs:26` | `DirtyBuildingState.java:38`, `InitialBuildingState.java:23`, `IncrementalInMemoryNodeEntry.java:446` | Abstract class + subclasses are one struct with a `DirtyBuildingStateKind`. |
| `dirty_building_state.rs:244` | `DirtyBuildingState.java:178-183` | `signalDep` cannot borrow the entry that owns it: the entry makes the first check (`checkState(isEvaluating(), "%s %s", entry, child)`, `incremental_in_memory_node_entry.rs:356`, `non_incremental_in_memory_node_entry.rs:205`) and passes `getNumTemporaryDirectDeps()` (a pure read) instead of itself. |
| `dirty_building_state.rs:287` | `DirtyBuildingState.java:214-219` | `unchangedFromLastBuild` takes the node's key (D10). `instanceof NotComparableSkyValue` is "is an `ErrorInfoValue`"; for other values the key's `values_equal` answers. |
| `in_memory_node_entry.rs:21` | `InMemoryNodeEntry.java:24`, `InMemoryGraphImpl.java:195,316` | The interface with two engine implementations is an enum chosen by `keep_edges`. |
| `in_memory_node_entry.rs:70`, `incremental_in_memory_node_entry.rs:32`, `:129` | `IncrementalInMemoryNodeEntry.java:46, 102-106, 119` | Done deps are not compressed (memory format). |
| `incremental_in_memory_node_entry.rs:42`, `reverse_deps_utility.rs:144`, `:214`, `:522` | `ReverseDepsUtility.java:47-57, 114-138, 362-373` | Reverse deps are always a `Vec` (Java: bare key / array / ArrayList by size). "Single reverse dep" is `len() == 1`, exactly Java's condition since every writer stores one element bare. Order kept. |
| `reverse_deps_utility.rs:56` | `KeyToConsolidate.java:34-175` | The bare encoding is kept (its meaning depends on `opToStoreBare` at decode time); interning and subclass layout dropped. |
| `reverse_deps_utility.rs:144` | `CompactHashSet` | Sets are `IndexSet`; a removal is `swap_remove`, which is `CompactHashSet.remove`'s order (last element moves into the hole, `lib/collect/compacthashset/CompactHashSet.java:372-430`). |
| `reverse_deps_utility.rs:270` | `ReverseDepsUtility.java:170-178` | Kept Java's bug (collects first occurrences, not duplicates); the `HashSet` order of that crash message is not reproduced (java-hash-order, crash text only). |
| `reverse_deps_utility.rs:294` | `ReverseDepsUtility.java:188-281` | `consolidateDataAndReturnNewElements(entry, mutateObject)` split into a pure computation and the mutation, so `returnNewElements` takes `&self`. Same checks, same order. |
| `reverse_deps_utility.rs:563` | `ReverseDepsUtility.java:403-409` | `toString` prints the `Vec` (Java prints the raw object; a `SkyKey[]` prints an identity hash). `singleReverseDep` is always false, as in Java (it passes the entry). |
| `abstract_in_memory_node_entry.rs:32` | `AbstractInMemoryNodeEntry.java:65` | Abstract class = fields struct (`base`) + trait with provided methods (template methods `getNumTemporaryDirectDeps`, `getTemporaryDirectDeps`). |
| `abstract_in_memory_node_entry.rs:89` | same | `synchronized` is the caller's lock. |
| `abstract_in_memory_node_entry.rs:133` | `AbstractInMemoryNodeEntry.java:98-103` | `key.supportsPartialReevaluation()` is false (no such method on Rust keys; partial reevaluation not ported). |
| `abstract_in_memory_node_entry.rs:261` | `AbstractInMemoryNodeEntry.java:208-219` | `newGroupedDeps` is always a plain `GroupedDeps` (`skipsBatchPrefetch` is not on Rust keys; Java default false). |
| `non_incremental_in_memory_node_entry.rs:321` | `NonIncrementalInMemoryNodeEntry.java:240` | `NonIncrementalBuildingState` wraps an `InitialBuildingState` (`base`) instead of extending it. |
| `non_incremental_in_memory_node_entry.rs:348` | `NonIncrementalInMemoryNodeEntry.java:246-251` | The entry passes a new empty `GroupedDeps` instead of itself (`entry.newGroupedDeps()`); used only when none exists. |
| `incremental_in_memory_node_entry.rs:392`, `non_incremental_in_memory_node_entry.rs:173` | `IncrementalInMemoryNodeEntry.java:291-292, 302-308, 328-330`, `NonIncrementalInMemoryNodeEntry.java:145-156` | Every `markDirty(REWIND)` fails with `unported!`, including Java's no-op cases (not done, or done in error). |
| `tests/in_memory_node_entry_test.rs:30` | `InMemoryNodeEntryTest.IntegerValue` | Test values' equality is on the test key (`TestKey::values_equal`, `IntegerValue` by value; an anonymous `new SkyValue() {}` is another type, never equal). |
| `tests/in_memory_node_entry_test.rs:124` | `ErrorInfo.equals` | `getErrorInfo()).isEqualTo(errorInfo)` compares `Debug` output (no `ErrorInfo.equals` port). |
| `tests/incremental_in_memory_node_entry_test.rs:1117` | `GroupedDeps.decompress` | Identity (no compression). |

Not a `// PORT:` site but worth knowing: `IntVersion::next` uses `wrapping_add` (Java `long` overflow).

## (c) `unported!` sites

- `src/value_with_metadata.rs:240` `ErrorInfoValue.equals` (needs `ErrorInfo.equals`; unreachable from `unchangedFromLastBuild`, which returns false first for an error value).
- `src/abstract_in_memory_node_entry.rs:205` `AbstractInMemoryNodeEntry.addExternalDep`.
- `src/incremental_in_memory_node_entry.rs:395` `IncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)`.
- `src/incremental_in_memory_node_entry.rs:541` `IncrementalInMemoryNodeEntry.resetEvaluationFromScratch`.
- `src/incremental_in_memory_node_entry.rs:556` `IncrementalInMemoryNodeEntry.clearDirectDepsForSkyfocus`.
- `src/non_incremental_in_memory_node_entry.rs:158` `NonIncrementalInMemoryNodeEntry.resetEvaluationFromScratch`.
- `src/non_incremental_in_memory_node_entry.rs:175` `NonIncrementalInMemoryNodeEntry.markDirty(DirtyType.REWIND)` (after Java's `checkArgument(dirtyType == REWIND, "Unexpected dirty type: %s")`).

## (d) Tests

Run: `scripts/cargo-capped test -p blaze_skyframe` in the worktree: 269 passed, 0 failed,
221 ignored (whole crate; before this work 204 passed, 124 ignored, all still passing).
`scripts/cargo-capped clippy -p blaze_skyframe --all-targets -- -D warnings`: clean.

Java's `@TestParameter boolean isPartialReevaluation` doubles every case of the three entry
classes; the `true` run is an ignored stub `<name>[_kind]_partial_reevaluation` ("partial
reevaluation (isPartialReevaluation = true) is not ported (docs/decisions.md 2026-11-19)").
Parameterized cases (`Transience`, `valueChanges`) whose body is ignored are one stub each.

| Java test class | Rust file | Cases | Passing | Ignored |
| --- | --- | --- | --- | --- |
| `JT:InMemoryNodeEntryTest` (24 cases) on the incremental entry | `tests/incremental_in_memory_node_entry_test.rs` (`<name>_incremental`), bodies in `tests/in_memory_node_entry_test.rs` | 48 | 16 | 32 |
| `JT:InMemoryNodeEntryTest` (24 cases) on the non-incremental entry | `tests/non_incremental_in_memory_node_entry_test.rs` (`<name>_non_incremental`) | 48 | 16 | 32 |
| `JT:IncrementalInMemoryNodeEntryTest` own (29 cases) | `tests/incremental_in_memory_node_entry_test.rs` | 58 | 25 | 33 |
| `JT:NonIncrementalInMemoryNodeEntryTest` own | — | 0 | 0 | 0 |
| `JT:ReverseDepsUtilityTest` (7 cases, each looping `numElements` 0..20) | `tests/reverse_deps_utility_test.rs` | 7 | 7 | 0 |
| (extra) `InMemoryNodeEntry` is `Send` | `tests/in_memory_node_entry_test.rs` | 1 | 1 | 0 |

Ignored with `isPartialReevaluation = false`, per entry kind (`_incremental` and
`_non_incremental` each): `signal_external_dep` (external deps not ported); `reset_lifecycle`,
`reset_twice_more_deps_requested_before_first_reset`, `rewinding_lifecycle`,
`reset_after_rewind`, `concurrent_rewinding_allowed`, `rewind_errorful_node_tolerated_but_no_op`
(rewinding / `resetEvaluationFromScratch` not ported);
`skips_batch_prefetch_test_temporary_deps_contains_hash_set` (`skipsBatchPrefetch` and
`GroupedDeps.WithHashSet` not ported). That is 8 per kind; the other 24 ignored per kind are the
partial-reevaluation stubs.

Ignored `IncrementalInMemoryNodeEntryTest` own cases: `get_all_direct_deps_for_incomplete_node_after_reset`,
`reset_on_dirty_node`, `rewind_on_incremental_build`, `rewind_on_dirty_node_ignored` (rewinding /
resets), plus the 29 `<name>_partial_reevaluation` stubs.

No Java assertion had to change; every ported case passes against the port.
