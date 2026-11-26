# I2 result: value equality and hermeticity of every key

Implementer: task `i2`, 2026-11-25. Tree: HEAD `aa55920` (crate code of `e8b79c0`) plus the
uncommitted work of this task and of other agents. Survey and preliminary verdicts:
`i2-worklist.md`. `J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`.

## What changed in the engine

- `Key::HERMETICITY: FunctionHermeticity` is required (I1's `Option` default `None` is gone), so
  the "Nodes with hermetic functions cannot be marked 'changed'" check runs for every key
  (`J:com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:467`). Test keys declare
  Java's test hermeticity (`SkyFunctionName.FOR_TESTING` and the tests' own names are
  `createHermetic`); the engine test key `Flag`, which tests re-inject, is non-hermetic.
- `blaze_skyframe::precomputed` (cross-group item X1): one `PrecomputedKey` / `PrecomputedValue`
  / `Precomputed<T: PartialEq>` for every crate. A value carries its type's `PartialEq`
  (`J:com/google/devtools/build/lib/skyframe/PrecomputedValue.java:128`), so re-injecting an
  equal variable no longer dirties its readers (`pruneInjectedValues`). `blaze_sky_loading` and
  `blaze_bzlmod` re-export the types and keep their variables; the bzlmod copy of the key type is
  gone, so `starlark_semantics` is one node (both crates injected the same value into the same
  builder, the later one winning).
- `blaze_skyframe::testing`: `parent_rebuilt::<K>(key, old, new)` evaluates a parent over a key
  whose function returns `old`, recomputes it to `new`, and tells whether the parent ran again;
  `values_equal_tester::<K>` is Guava's `EqualsTester` over `Key::values_equal`.
- Review S2-1: a dirty node's old dep missing from the graph fails with Java's texts
  (`AbstractParallelEvaluator.java:394`, `:1022`). S2-2: the interrupt check after invalidation is
  inside the invalidation branch (Java's invalidators throw for a pending interrupt); with nothing
  to invalidate, cached roots are returned (`ParallelEvaluator.java:641`).

## Per key

`=` opts in with the value type's `==`; `fn` a hand-written `values_equal`; `-` the default
(never equal). Hermeticity: H hermetic, S semi-hermetic, N non-hermetic.

### Group 1: `blaze_sky_loading` (32, with the shared `PrecomputedKey`)

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `FileStateKey` | N | = | per-subclass `equals`; agrees |
| `FileKey` | S | = | Java's `SymlinkWithStoredChain` uses `instanceof` (prunes across two classes); Rust does not (prunes less) |
| `DirectoryListingStateKey` | N | = | `CompactSortedDirents.equals` |
| `DirectoryListingKey` | H | = | |
| `FileSymlinkCycleUniquenessKey`, `FileSymlinkInfiniteExpansionUniquenessKey` | H | = | `EmptySkyValue.INSTANCE` |
| `PackageLookupKey` | H | fn | `NoRepository` never equal (identity) |
| `ContainingPackageLookupKey` | H | fn | `NoContainingPackage` with a reason never equal; `NONE` equal |
| `LocalRepositoryLookupKey` | H | fn | `Successful` compares the name only |
| `IgnoredSubdirectoriesKey` | H | = | |
| `PrecomputedKey` (`blaze_skyframe`) | N | fn | the wrapped value's `PartialEq` |
| `RepositoryMappingKey` | H | = | record |
| `RepositoryDirectoryKey` | N | - | `NotComparableSkyValue` |
| `ProjectFilesLookupKey` | H | = | |
| `ProjectKey`, `StarlarkBuiltinsKey`, `BzlLoadKey`, `TransitiveTargetKey` | H | - | no `equals` |
| `BzlCompileKey`, `PackageKey`, `PackagePieceForBuildFileKey`, `EvalMacroKey` | H | - | `NotComparableSkyValue` |
| `PackageDeclarationsKey` | H | fn | metadata, declarations, semantics, main mapping |
| `MacroInstanceKey` | H | - | **deferred**: `MacroInstance` has no `PartialEq` (Starlark attribute values) |
| `NonFinalizerPackagePiecesKey` | H | - | **deferred**: needs `MacroInstance` equality |
| `GlobsKey` | H | = | |
| `RecursivePkgKey` | H | fn | **differs from survey**: only Java's `EMPTY` singleton (no packages, no errors) is equal |
| `PackageErrorMessageKey` | H | = | |
| `PackageErrorKey` | H | - | never a value |
| `RepoFileKey` | H | - | **deferred**: Java's `equals` on Starlark values is not ported as one function; `RepoPackageArgsKey` above prunes |
| `RepoPackageArgsKey` | H | fn | `PackageArgs` |
| `TargetPatternKey` | S | - | no `equals` |

### Group 2: `blaze_bzlmod` 12 + `blaze_repo_rules` 6

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `ModuleFileKey` | N | fn | `==` and the registry compared by identity (`Registry` is `NotComparableSkyValue`); `InterimModule::eq` unchanged |
| `BazelModuleResolutionKey` | H | fn | `==` and each unpruned module's registry by identity |
| `BazelDepGraphKey`, `BazelLockFileKey`, `SingleExtensionKey`, `SingleExtensionUsagesKey`, `ModuleExtensionRepoMappingEntriesKey`, `DirectoryTreeDigestKey`, `BazelFetchAllKey` | H | = | |
| `RepoSpecKey`, `YankedVersionsKey`, `SingleExtensionEvalKey`, `ClientEnvironmentKey`, `RepoEnvironmentKey` | N | = | |
| `BazelModuleInspectorKey`, `BazelModTidyKey` | H | fn | equal only when neither has errors (exceptions are identity) |
| `RegistryKey` | N | - | `NotComparableSkyValue` |
| `RepoDefinitionKey` | H | - | `NotComparableSkyValue`; its `PartialEq` is not used |

### Group 3: `blaze_build_config` (7)

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `PlatformValueKey` | H | fn | `PlatformInfo.equals` through `OpaquePlatformInfo::java_equals` (captured by `OpaquePlatformInfo::new`), plus the parsed flags; an oracle-recorded value (no `PlatformInfo`) is never equal |
| `BuildConfigurationKey`, `BuildConfigurationKeyValueKey`, `ParsedFlagsKey`, `PlatformMappingSkyKey` | H | = | |
| `BaselineOptionsKey` | N | = | |
| `BuildOptionsScopeKey` | H | - | no `equals` |

### Group 4: `blaze_analysis` 7 + `blaze_sky_analysis` 4

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `ToolchainContextKey`, `StarlarkBuildSettingsDetailsKey`, `RegisteredExecutionPlatformsKey`, `RegisteredToolchainsKey`, `SingleToolchainResolutionKey` | H | = | Rust compares some sets in order (prunes less) |
| `ConfiguredTargetKey`, `AspectKey` | H | - | `NotComparableSkyValue` |
| `WorkspaceStatusKey`, `TopLevelAspectsKey`, `LoadAspectsKey`, `FlagSetKey` (new since the survey) | H | - | no `equals` |

### Group 5: `blaze_sky_exec` (9)

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `ActionExecutionKey` | S | fn | **narrower than Java**: file values only; tree artifacts, rich data and remote file values never equal |
| `ArtifactKey` | H | fn | **narrower than Java**: local `FileArtifactValue`s only; trees, missing and remote never equal |
| `TargetCompletionKey`, `AspectCompletionKey`, `TestCompletionKey` | H | fn | Java singletons: always equal |
| `DirectoryArtifactTraversalRequest` | H | = | |
| `ArtifactNestedSetKey` | H | - | `NotComparableSkyValue` |
| `BuildDriverKey` | N | - | no `equals` |
| `ActionTemplateExpansionKey` | H | - | no `equals` |

### Group 6: `blaze_sky_top` (6) and `ErrorTransienceKey`

| Key | Herm. | Equality | Note |
| --- | --- | --- | --- |
| `ActionLookupConflictFindingKey`, `TopLevelActionLookupConflictFindingKey` | H | fn | singleton: always equal |
| `TargetPatternPhaseKey` | N | fn | without `non_expanded_labels`, as Java |
| `TargetPatternErrorKey` | H | - | never a value |
| `TestExpansionKey`, `TestsForTargetPatternKey` | H | - | no `equals` |
| `ErrorTransienceKey` (`blaze_skyframe`) | N | - | never equal, as Java |
| `RuleContextForTestingKey` (test harness, `src/testing`) | N | - | no Java key; reads harness state |

## Open (ranked)

1. `MacroInstance` / `AttributeContainer` equality (Java `MacroInstance.equals`,
   `RuleOrMacroInstance.equalsHelper`), then `MacroInstanceKey` and
   `NonFinalizerPackagePiecesKey` (macro instances by value, pieces and targets by identity).
2. `TreeArtifactValue.equals` and `RunfilesArtifactValue.equals` (ignoring the runfiles tree) in
   `blaze_exec`, and a remote `FileArtifactValue` that keeps Java's one-way class check; then
   `ActionExecutionKey` and `ArtifactKey` prune as Java.
3. Java `equals` of Starlark values for `RepoFileValue`'s map.
4. Keys sharing a function name must declare one hermeticity (Java asserts it when interning the
   name); nothing checks this in Rust (today only `PACKAGE`, both `Hermetic`).
