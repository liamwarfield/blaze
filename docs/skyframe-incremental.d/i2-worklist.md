# I2 work list: SkyFunctions and value types for a kept graph

Surveyor: I2-survey (read-only). Tree: `/home/liamwarfield/.t3/projects/blaze` at HEAD
`4f9291238f7a0a99cb35285087c401a64df5f29b` plus the uncommitted work of other agents present on
2026-11-19. Line numbers are taken from that tree. No file in the tree was edited and no build was run.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`.

Method: the key list came from
`grep -rnE "impl\s*(<[^>]*>)?\s*(blaze_skyframe::)?Key\s+for" crates --include=*.rs`, which returns
**100 hits**. **16 are excluded as tests**: `crates/blaze_skyframe/src/tests.rs` (10),
`crates/blaze_skyframe/src/tests/parallel_evaluator_test.rs` (2),
`crates/blaze_skyframe/src/error_info.rs:227` (inside `#[cfg(test)]` at :218),
`crates/blaze_sky_top/src/cycles_reporter.rs:552` (`#[cfg(test)]` at :544),
`crates/blaze_sky_analysis/tests/platform_mapping_function_parser_test.rs:44`, and
`crates/blaze_sky_exec/tests/package_root_resolver_with_environment_test.rs:38`. That leaves
**84 production keys**. A cross-check over `const FUNCTION: SkyFunctionName` gave 87 non-test lines: the
84, the two test constants in `error_info.rs` and `cycles_reporter.rs`, and the trait declaration in
`blaze_skyframe/src/lib.rs:196`. No key types are made by a macro, and there are no production keys in
`blaze_cmdline`, `blaze_vfs`, `blaze_actions`, `blaze_packages`, `blaze_exec`, `blaze_buildtool`,
`blaze_query`, `blaze_cquery`, `blaze_remote` or `blaze_runtime`. The one key outside the six groups is
`ErrorTransienceKey` (`crates/blaze_skyframe/src/evaluator.rs:46`). It is listed with group 6, and I
recommend that I1 owns it, because the I1-B worktree already declares its `HERMETICITY`.

The survey was split across six read-only helpers, one per crate group (group 1 in two halves). I
merged their drafts, spot-checked them against the Java and the Rust, and corrected one claim (the
`BY_FILENAME` row of group 2). The spot checks were `ProjectValue` (no `equals`),
`LocalRepositoryLookupValue.Successful.equals`, `TargetPatternPhaseValue.equals:118`,
`InterimModule::eq` and `BY_FILENAME` readers. Every verdict is **preliminary: the I2 worker verifies
field by field**.

The I1 API (`Key::values_equal`, `Key::HERMETICITY`) is **not in the shared tree yet**. `grep -rn
"values_equal\|HERMETICITY" crates/blaze_skyframe/src` finds nothing at HEAD. It exists in worktree
`i1-b` (`docs/skyframe-incremental.d/support-and-model.md`). No key declares `HERMETICITY` today.

Verdict vocabulary, as in the task:
- `EQ-OK`: opt in with today's `==`.
- `EQ-WRITE`: Java has value equality, and Rust must write `values_equal` or a `PartialEq` first.
- `IDENTITY`: leave the default.
- `RUST-STRONGER`: Rust's `PartialEq` would prune where Java never does, so do not opt in.
- `MIXED`: the key is right for some variants and wrong for others.

Where a row says "Rust is stricter (compares order)", Rust's `==` says "unequal" in cases where Java
says "equal". That only costs a prune and is never a wrong build. A port should still match Java
exactly where it is cheap.

Java's pruning rule (`J:com/google/devtools/build/skyframe/DirtyBuildingState.java:214-218`) is
`!(newValue instanceof NotComparableSkyValue) && lastBuildValue.equals(newValue)`. Injected values
are pruned by `equals` in `AbstractInMemoryMemoizingEvaluator.java:432-443` (`pruneInjectedValues`).

**Registration does not matter for the opt-in.** Several keys are registered with an untyped
`.function(..)` (`BUILD_CONFIGURATION` at `crates/blaze_build_config/src/skyframe.rs:1293`, and both
conflict-finding functions at `crates/blaze_sky_top/src/action_lookup_conflict_finding_function.rs:376-383`).
Two helpers raised this. Per `docs/skyframe-incremental.md` section 3, `values_equal` is reached
through `DynKey` from the `SkyKey`, so it is declared on the key type no matter how the function is
registered. I1 should confirm this when the evaluator calls it. A related I1 requirement:
`ErrorTransienceValue.equals` is false even for the same object
(`J:com/google/devtools/build/skyframe/ErrorTransienceValue.java:59`), so the evaluator must not
short-cut `values_equal` with `Arc::ptr_eq`.

Ownership rule I recommend: a group owns the `impl Key` blocks and the SkyFunction bodies in its
crates. Shared types defined in lower crates (`blaze_packages`, `blaze_exec`, `blaze_bzlmod_model`,
`blaze_actions`) are assigned in "Cross-group items". `crates/blaze_sky_top/src/skyframe_executor.rs`
is the registration site for every group, and group 6 owns it. Other groups send group 6 the
registration change they need and do not edit the file themselves.

## Group 1: blaze_sky_loading (32 keys)

Drafted in two halves: keys 1-18 with sections B and C for the whole crate, then keys 19-32 with sections D and E for the whole crate.

Helper draft: **G1a: blaze_sky_loading, part 1 (18 keys; sections A, B, C for the whole crate)**

Tree: HEAD `4f92912` plus uncommitted work, read 2026-11-19. Rust paths are relative to
`crates/blaze_sky_loading/src/` unless they start with `crates/`. `J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`.

Java change pruning rule used for every row: `J:com/google/devtools/build/skyframe/DirtyBuildingState.java:214-218`
`unchangedFromLastBuild` = `!(newValue instanceof NotComparableSkyValue) && lastBuildValue.equals(newValue)`
(old.equals(new), so an asymmetric Java `equals` is read with the old value as receiver).
Injected values (PRECOMPUTED) are also pruned by `equals`:
`J:com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:432,443` `pruneInjectedValues`.

### A. Keys and values

| # | Rust key | FUNCTION | Rust value type | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `file_state.rs:46` FileStateKey | FILE_STATE | `actions/file_state_value.rs:147` FileStateValue (enum; `FileContentsProxy` `:80` derived) | derived (`:146`; `FileContentsProxy` derived `:79`) | `J:com/google/devtools/build/lib/vfs/FileStateKey.java:25` | `J:com/google/devtools/build/lib/actions/FileStateValue.java:67` abstract; `equals` per subclass: WithDigest `:335` (size + `Arrays.equals(digest)`), WithContentsProxy `:407` (size + proxy), WithMetadata `:482` (not ported), Special `:557` (proxy), Directory `:608` (any instance), Symlink `:645` (target), Nonexistent `:697` (any instance). `FileContentsProxy.equals` `J:.../actions/FileContentsProxy.java:54` (ctime, mtime, nodeId) | NonHermetic `J:com/google/devtools/build/lib/vfs/FileStateKey.java:26` | EQ-OK (prelim.): variant-by-variant same fields; `Vec<u8>` digest == `Arrays.equals`; Directory/Nonexistent unit variants = Java "instanceof" equality. Create path keeps Java's digest-vs-proxy choice (`actions/file_state_value.rs:254-265`), so a `touch` changes the value as in Java. `RegularFileStateValueWithMetadata` not ported (no variant) |
| 2 | `file.rs:50` FileKey | FILE | `actions/file_value.rs:22` FileValue (enum; `Regular(FileStateValue)` + 6 variants) | derived (`:21`) | `J:com/google/devtools/build/lib/skyframe/FileKey.java:27` | `J:com/google/devtools/build/lib/actions/FileValue.java:49` interface; Regular = the FileStateValue itself (`:226`, row 1); `equals`: DifferentRealPathWithUnboundedAncestorExpansion `:294` (getClass, 5 fields), DifferentRealPathWithStoredChain `:373` (getClass, 3 fields), DifferentRealPathWithoutStoredChain `:443` (getClass, 2 fields), SymlinkWithUnboundedAncestorExpansion `:515` (getClass, 6 fields), SymlinkWithStoredChain `:575` (**instanceof**, 4 fields), SymlinkWithoutStoredChain `:629` (instanceof, 3 fields) | SemiHermetic `J:.../skyframe/SkyFunctions.java:53` | EQ-OK (prelim.): same fields per variant; `RootedPath`/`Root`/`Path` have Java-ported manual `eq` (`crates/blaze_vfs/src/rooted_path.rs:103`, `root.rs:172`, `path.rs:519`). Only difference: Java old `SymlinkWithStoredChain`.equals(new `SymlinkWithUnboundedAncestorExpansion`) can be true via `instanceof` (`:579`); Rust says false (less pruning, never wrong) |
| 3 | `directory_listing.rs:105` DirectoryListingStateKey | DIRECTORY_LISTING_STATE | `directory_listing.rs:67` DirectoryListingStateValue (wraps `Dirents` `:25`, sorted `Vec<Dirent>`) | derived (`:66`, `Dirents` `:24`, `Dirent` derived `crates/blaze_vfs/src/dirent.rs:34`) | `J:.../skyframe/DirectoryListingStateValue.java:56` (`Key`) | `J:.../skyframe/DirectoryListingStateValue.java:35`, `equals` `:99` -> `CompactSortedDirents.equals` `J:.../skyframe/CompactSortedDirents.java:56` (names array + packed types) | NonHermetic `J:.../skyframe/SkyFunctions.java:33` | EQ-OK (prelim.): both sorted by name, compare (name, type) element-wise |
| 4 | `directory_listing.rs:237` DirectoryListingKey | DIRECTORY_LISTING | `directory_listing.rs:169` DirectoryListingValue (enum; `Arc<DirectoryListingStateValue>`) | derived (`:168`); `Arc<T>` `==` compares contents | `J:.../skyframe/DirectoryListingKey.java:27` | `J:.../skyframe/DirectoryListingValue.java:37` abstract; Regular `equals` `:85` (state value), DifferentRealPath `:124` (real path + state value) | Hermetic `J:.../skyframe/SkyFunctions.java:35` | EQ-OK (prelim.) |
| 5 | `file_chain_uniqueness.rs:72` FileSymlinkCycleUniquenessKey | FILE_SYMLINK_CYCLE_UNIQUENESS | `file_chain_uniqueness.rs:18` EmptySkyValue (unit struct) | derived (`:17`) | `J:com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:40` | `J:com/google/devtools/build/skyframe/EmptySkyValue.java:22`, no equals: identity, but only `INSTANCE` (`:24`) is ever returned -> always equal (enum singleton) | Hermetic `J:com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:32` | EQ-OK (prelim.): singleton. (The function exists for its event; see G5 elsewhere) |
| 6 | `file_chain_uniqueness.rs:95` FileSymlinkInfiniteExpansionUniquenessKey | FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS | `file_chain_uniqueness.rs:18` EmptySkyValue | derived (`:17`) | `J:com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:41` | as row 5 | Hermetic `J:com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:33` | EQ-OK (prelim.): singleton |
| 7 | `package_lookup.rs:303` PackageLookupKey | PACKAGE_LOOKUP | `package_lookup.rs:54` PackageLookupValue (enum, 6 variants) | derived (`:53`) | `J:.../skyframe/PackageLookupValue.java:130` | `J:.../skyframe/PackageLookupValue.java:47` abstract. Success `equals` `:207` (root.equals, buildFileName ==); NoBuildFile singleton `NO_BUILD_FILE_VALUE` `:50` (private ctor, no equals); InvalidName `:274` (errorMsg); IncorrectRepositoryReference `:329` (both ids); Deleted singleton `DELETED_PACKAGE_VALUE` `:54`; **NoRepository `:371`: no equals: identity**, new instance per compute | Hermetic `J:.../skyframe/SkyFunctions.java:41` (comment `:38-39`: hermetic because `SequencedSkyframeExecutor` deletes affected PACKAGE_LOOKUP nodes when `--deleted_packages` changes) | MIXED (prelim.): 5 variants agree; `NoRepository { repository_name, reason }` derived eq is RUST-STRONGER (Java never prunes it). Needs a manual `values_equal` that returns false for `NoRepository` |
| 8 | `containing_package_lookup.rs:167` ContainingPackageLookupKey | CONTAINING_PACKAGE_LOOKUP | `containing_package_lookup.rs:20` ContainingPackageLookupValue (enum) | derived (`:19`) | `J:.../skyframe/ContainingPackageLookupValue.java:110` | `J:.../skyframe/ContainingPackageLookupValue.java:37` abstract. ContainingPackage `equals` `:211` (pkg id + root). NoContainingPackage `:147`: **no equals: identity**; `NONE` singleton `:39` (returned at `ContainingPackageLookupFunction.java:58`), else `noContainingPackage(reason)` new instance (`ContainingPackageLookupFunction.java:54`) | Hermetic `J:.../skyframe/SkyFunctions.java:43` | MIXED (prelim.): `ContainingPackage` and `NONE` (`reason: None`) agree; `NoContainingPackage { reason: Some(_) }` derived eq is RUST-STRONGER. Manual `values_equal`: false for `reason: Some` |
| 9 | `local_repository_lookup.rs:107` LocalRepositoryLookupKey | LOCAL_REPOSITORY_LOOKUP | `local_repository_lookup.rs:19` LocalRepositoryLookupValue (enum) | derived (`:18`) | `J:.../skyframe/LocalRepositoryLookupValue.java:40` | `J:.../skyframe/LocalRepositoryLookupValue.java:32` abstract. Main `equals` `:131` (instanceof); Successful `:174` (**repositoryName only; `path` ignored**); NotFound `:214` (instanceof) | Hermetic `J:.../skyframe/SkyFunctions.java:138` | EQ-WRITE (prelim.): Java has value equality but ignores `path`; Rust derive also compares `path` (weaker: never prunes wrongly, but deviates). Write manual eq: `Successful` compares `repository_name` only. No blocking field types |
| 10 | `ignored_subdirectories.rs:247` IgnoredSubdirectoriesKey | IGNORED_SUBDIRECTORIES | `ignored_subdirectories.rs:182` IgnoredSubdirectoriesValue (wraps `IgnoredSubdirectories` `:30`: `IndexSet<PathFragment>` prefixes, `Vec<String>` patterns) | derived (`:181`, inner `:29`; manual `Hash` `:157`) | `J:.../skyframe/IgnoredSubdirectoriesValue.java:88` | `J:.../skyframe/IgnoredSubdirectoriesValue.java:30`, `equals` `:74` -> `J:com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:188` (prefixes `ImmutableSet` set-equality, patterns `ImmutableList` ordered) | Hermetic `J:.../skyframe/SkyFunctions.java:85` | EQ-OK (prelim.): `IndexSet ==` is order-insensitive like `ImmutableSet.equals`; `Vec ==` ordered like `ImmutableList` |
| 11 | `precomputed.rs:59` PrecomputedKey | PRECOMPUTED | `precomputed.rs:21` PrecomputedValue (`Arc<dyn Any + Send + Sync>`) | none (`#[derive(Clone)]` `:20`) | `J:.../skyframe/PrecomputedValue.java:191` | `J:.../skyframe/PrecomputedValue.java:45`, `equals` `:128` delegates to the wrapped object's `equals` | NonHermetic `J:.../skyframe/SkyFunctions.java:23` | EQ-WRITE (prelim.): blocked by `Arc<dyn Any>`; needs an equality fn captured by `Precomputed<T>::new`/`set` (`T: PartialEq`, and each `T` checked against its Java `equals`: `StarlarkSemantics`, `PathPackageLocator`, `IndexMap<String,PathFragment>`, `AutoloadSymbols`, `RuleVisibility`, `ConfigSettingVisibilityPolicy`, `LazyMacroExpansionPackages` (derived `:163`), see `precomputed.rs:124-157`). Critical: Java's `pruneInjectedValues` uses this equality, so without it every injected precomputed dirties all readers each command |
| 12 | `repos.rs:176` RepositoryMappingKey | REPOSITORY_MAPPING | `repos.rs:41` RepositoryMappingValue (`Option<RepositoryMapping>`, 2 x `Option<String>`) | derived (`:40`); `RepositoryMapping` manual `crates/blaze_cmdline/src/repository_mapping.rs:171` (ptr_eq or entries + context_repo; `IndexMap ==` order-insensitive; cached inverse ignored) | `J:.../skyframe/RepositoryMappingValue.java:121` (AutoValue `Key`) | `J:.../skyframe/RepositoryMappingValue.java:47` **record** (repositoryMapping, associatedModuleName, associatedModuleVersion); `RepositoryMapping.equals` `J:com/google/devtools/build/lib/cmdline/RepositoryMapping.java:65` (entries + contextRepo) | Hermetic `J:.../skyframe/SkyFunctions.java:148` | EQ-OK (prelim.) |
| 13 | `repos.rs:221` RepositoryDirectoryKey | REPOSITORY_DIRECTORY | `repos.rs:187` RepositoryDirectoryValue (enum Success/Failure) | derived (`:186`) | `J:com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:78` | `J:com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:40` sealed interface **extends NotComparableSkyValue** (records `Success` `:56`, `Failure` `:65`) | NonHermetic `J:.../skyframe/SkyFunctions.java:134` | RUST-STRONGER (prelim.): Java comment `:33-38`: "we explicitly disable change pruning here by extending NotComparableSkyValue. The reason is that, after fetching a repo successfully, the resultant Success object does not capture the newly fetched contents of the repo ... Without change pruning, we force dependent SkyValues to be marked dirty whenever a repo is re-fetched." Must not opt in |
| 14 | `project_function.rs:61` ProjectFilesLookupKey | PROJECT_FILES_LOOKUP | `project_function.rs:70` ProjectFilesLookupValue (`Vec<Label>`) | derived (`:69`) | `J:.../skyframe/ProjectFilesLookupValue.java:95` | `J:.../skyframe/ProjectFilesLookupValue.java:34`, `equals` `:78` (projectFiles list) | Hermetic `J:.../skyframe/SkyFunctions.java:46` | EQ-OK (prelim.) |
| 15 | `project_function.rs:381` ProjectKey | PROJECT | `project_function.rs:285` ProjectValue (+ `BuildableUnit` `:219`) | derived (`:284`, `:218`) | `J:.../skyframe/ProjectValue.java:191` | `J:.../skyframe/ProjectValue.java:32` final class, **no equals: identity** (only `Key.equals` `:208`; `BuildableUnit` is AutoValue `:81` but the outer value is identity) | Hermetic `J:.../skyframe/SkyFunctions.java:44` | RUST-STRONGER (prelim.): Java never prunes PROJECT; no Java comment. Leave default |
| 16 | `starlark_builtins.rs:137` StarlarkBuiltinsKey | STARLARK_BUILTINS | `starlark_builtins.rs:34` StarlarkBuiltinsValue (`StarlarkEnv`s, digest, `Arc<StarlarkSemantics>`, `GlobalsCache`) | none | `J:.../skyframe/StarlarkBuiltinsValue.java:139` | `J:.../skyframe/StarlarkBuiltinsValue.java:34` final class, no equals: identity | Hermetic `J:.../skyframe/SkyFunctions.java:49` | IDENTITY (prelim.) |
| 17 | `bzl_compile.rs:174` BzlCompileKey | BZL_COMPILE | `bzl_compile.rs:183` BzlCompileValue (enum; `Arc<CompiledFile>`) | none (`#[derive(Clone, Debug)]` `:182`) | `J:.../skyframe/BzlCompileValue.java:160` | `J:.../skyframe/BzlCompileValue.java:51` **implements NotComparableSkyValue** (comment `:38-49`: "almost any change to a .bzl causes the BzlCompileValue to be recomputed ... Comparing syntax trees for equality is complex and expensive") | Hermetic `J:.../skyframe/SkyFunctions.java:47` | IDENTITY (prelim.) |
| 18 | `bzl_load.rs:228` BzlLoadKey | BZL_LOAD | `bzl_load.rs:236` BzlLoadValue (`FrozenModule`, digest, visibility, mappings, `Arc<BazelModuleContext>`) | none (`#[derive(Clone)]` `:235`) | `J:.../skyframe/BzlLoadValue.java:107` (sealed `Key`, subclasses `:217,273,307,332`) | `J:.../skyframe/BzlLoadValue.java:57` class, no equals: identity | Hermetic `J:.../skyframe/SkyFunctions.java:50` | IDENTITY (prelim.) |

Verdict counts (18): EQ-OK 9 (rows 1-6, 10, 12, 14); EQ-WRITE 2 (9, 11); MIXED 2 (7, 8);
RUST-STRONGER 2 (13, 15); IDENTITY 3 (16-18).

Notes on rows:
- Keys with manual `Eq`/`Hash`: `BzlCompileKey` manual `PartialEq` `bzl_compile.rs:140` (Java `BzlCompileValue.Key.equals` `J:.../BzlCompileValue.java:227`), `BzlLoadKey` manual `Hash` `bzl_load.rs:205`. Not audited further here (not in scope of A).
- Hermeticity is not represented in Rust (`sky_functions.rs:1-6`, "PORT: skipped, incremental"); every FUNCTION in this table has a Java creation site, all `createHermetic`/`createNonHermetic`/`createSemiHermetic` (no plain `SkyFunctionName.create`).
- Stale `// Java:` line markers seen while reading (S4; the classes are right, the lines are not): `ProjectValue.Key` Rust says `:155`, Java `:191`; `ProjectFilesLookupValue.Key` `:77` vs `:95`; `StarlarkBuiltinsValue.Key` `:137` vs `:139`; `BzlCompileValue.Key` `:151` vs `:160`; `BzlLoadValue` `:53` vs `:57`; `ContainingPackageLookupValue.Key` `:117` vs `:110`; `RepositoryDirectoryValue` `:37`/`Key :74` vs `:40`/`:78`; `IgnoredSubdirectoriesValue.Key` `:94` vs `:88`; `DirectoryListingKey` `:25` vs `:27`; `PackageLookupValue.Key` `:129` vs `:130`.

### B. Inputs that are not dependencies (whole crate)

No `env.client::<T>()` / `.client()` reader exists in the crate (grep, non-test src: 0 hits).
All captured inputs come from `LoadingFunctionsConfig` (`skyframe_executor.rs:89-122`) via
`register_loading_functions` (`skyframe_executor.rs:278-418`), plus statics.

| Captured item | Captured by (Rust) | Per-command? | Java | Class |
|---|---|---|---|---|
| `directories` `skyframe_executor.rs:90` | ExternalFilesHelper `file_state.rs:101`, FileFunction (`immutable_paths`) `file.rs:132`, BzlLoadConfig `bzl_load.rs:760`, RecursivePkgFunction `recursive_pkg.rs:470`, RepoFileFunction workspace root `repo_file.rs:143` | no | `SkyframeExecutor.directories` ctor arg `J:.../skyframe/SkyframeExecutor.java:718` | (a) |
| `pkg_locator: Arc<RwLock<PathPackageLocator>>` `:92`; written by `sync_package_loading` `:194` | ExternalFilesHelper (FILE_STATE, DLS) read at `file_state.rs:143`; FileFunction `file.rs:237`; PACKAGE via `SkyframePackageLocator` `package_function.rs:135,147` (non-Skyframe globber subpackage check) | yes (`--package_path`) | `AtomicReference<PathPackageLocator> pkgLocator` `J:.../skyframe/SkyframeExecutor.java:408`; `setPackageLocator` `:1759-1779` also injects `PATH_PACKAGE_LOCATOR` and on change calls `onPkgLocatorChange` = `invalidate(FILE_STATE, FILE, DIRECTORY_LISTING_STATE, PREPARE_DEPS_OF_PATTERN, TARGET_PATTERN, TARGET_PATTERN_PHASE)` `J:.../skyframe/SequencedSkyframeExecutor.java:352-364` | (c) cell; port the `onPkgLocatorChange` invalidation. (Also injected as `PATH_PACKAGE_LOCATOR` (b) at `skyframe_executor.rs:502`, read by TARGET_PATTERN `target_pattern_function.rs:453`) |
| `deleted_packages: Arc<RwLock<HashSet<PackageIdentifier>>>` `:94`; written `:200-204` | PackageLookupFunction `package_lookup.rs:329,367`; PACKAGE via `SkyframePackageLocator` `package_function.rs:136,144` | yes (`--deleted_packages`) | `AtomicReference deletedPackages` `J:.../skyframe/SkyframeExecutor.java:409`; `setDeletedPackages` `J:.../skyframe/SequencedSkyframeExecutor.java:373-385` **deletes** (not invalidates) PACKAGE_LOOKUP nodes for the symmetric difference | (c) cell; port the delete (marked `PORT: skipped, incremental` at `skyframe_executor.rs:200`) |
| `show_loading_progress: bool` `:118`; copied by value into `PackageFunctionConfig` `skyframe_executor.rs:330` / `package_function.rs:458`, read `package_function.rs:1088` | PACKAGE | yes (`--show_loading_progress`) | `AtomicBoolean showLoadingProgress` `J:.../skyframe/SkyframeExecutor.java:407`, set `:1595,1711`, shared by reference with PackageFunction (`:826`), read at compute `J:.../skyframe/PackageFunction.java:1088`; no invalidation | (c) cell without invalidation: Rust must share a cell (`Arc<AtomicBool>`) instead of copying the bool at registration, or a kept executor keeps the first command's value |
| `repo_contents_cache_path: Option<Path>` `:97` (set per command by `crates/blaze_sky_top/src/skyframe_executor.rs:306`) | ExternalFilesHelper `file_state.rs:105` (FileType `RepoContentsCacheDirs`) | yes (`--repo_contents_cache`) | `Supplier<Path> repoContentsCachePathSupplier` = `repositoryCache.getRepoContentsCache()::getPath` `J:com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:217`, read at each call; no invalidation | (c) cell/supplier without invalidation (today a by-value copy) |
| `external_repos: Arc<dyn ExternalRepos>` `:106` (trait `repos.rs:243`) | RepositoryMappingFunction `repos.rs:288`, RepositoryDirectoryFunction `repos.rs:375`, StarlarkBuiltinsFunction `starlark_builtins.rs:273` | the object is built per command by the owner (blaze_sky_top / blaze_repo_rules) | the seam stands for Java SkyFunctions (`RepositoryMappingFunction.computeInternal`, `RepositoryFetchFunction`) whose own captured state is audited with blaze_bzlmod / blaze_repo_rules | (a) as a seam; its contents belong to the G1 rows of those crates |
| `external_file_action` `:95`, `cross_repository_label_violation_strategy` `:99`, `build_files_by_priority` `:100`, `tools_repository` `:102`, `native_registry` `:108`, `digest_function` `:110`, `bazel_starlark_environment` `:112`, `rule_class_provider` `:115`, `package_factory` `:118` (PackageSettings, `NoopValidator`), `fragment_names` `:122`, `should_use_repo_dot_bazel` (`package_function.rs:460`) | PackageLookupFunction, BzlCompileFunction `bzl_compile.rs:319`, BzlLoadConfig `bzl_load.rs:759`, RepoFileFunction `repo_file.rs:142`, StarlarkBuiltinsFunction `starlark_builtins.rs:270`, PackageFunctionConfig `package_function.rs:452`, EvalMacroFunction `package_pieces.rs:469` | no | `SkyframeExecutor` ctor args / `ruleClassProvider` / `BazelSkyframeExecutorConstants` (`J:.../skyframe/SkyframeExecutor.java:740-760`) | (a) |
| `BzlLoadFunction.bzl_compile_cache: Mutex<HashMap>` `bzl_load.rs:775` (read `:1020`, insert `:1032`, remove `:843`) | BZL_LOAD | cache | `InliningAndCachingGetter.bzlCompileCache` `J:.../skyframe/BzlLoadFunction.java:1480-1522`; the shared cache `SkyframeExecutor.java:393` is `invalidateAll()`ed in `preparePackageLoading` `:1738` each command | (c)-like: cache cell with per-command reset; Rust has no reset (fine only while one evaluator per command) |
| static `PACKAGE_PROGRESS` `package_progress_receiver.rs:34`, written `package_function.rs:1161` | PACKAGE | per build | `packageProgress` field `J:.../skyframe/SkyframeExecutor.java:397`, `reset()` `:1742` in `preparePackageLoading` | (d)/(c): per-build counter, needs reset per command (already listed in plan 3.1) |
| static `KEEP_ALIVE: Mutex<Vec<FrozenModule>>` `bzl_load.rs:732`, push `:1516` | BZL_LOAD | grows | no Java counterpart (GC) | not an input; memory (plan 3.3) |
| static `BUNDLED_FS: OnceLock` `bundled_fs.rs:376` | FILE_STATE classification (`file_state.rs:136`) | no | embedded builtins file system | (a) |
| `Precomputed` statics `precomputed.rs:124-157` (STARLARK_SEMANTICS, PATH_PACKAGE_LOCATOR, REPOSITORY_OVERRIDES, AUTOLOAD_SYMBOLS, DEFAULT_VISIBILITY, CONFIG_SETTING_VISIBILITY_POLICY, LAZY_MACRO_EXPANSION_PACKAGES) | readers through `env` | yes | `PrecomputedValue.java:79-107`, `RepoDefinitionFunction.java:58`, `AutoloadSymbols.java:99` | (b) already deps; injected by `inject_loading_precomputed_impl` `skyframe_executor.rs:481-508`. Need row 11 equality |

Per-command values passed only as call arguments, not captured: `PackageOptions` to
`sync_package_loading` (`skyframe_executor.rs:183`) and `inject_loading_precomputed_with_package_options` (`:446`). No `now()`, `SystemTime`, `Instant`, or `std::env::var` in non-test src.

### C. File system reads (whole crate, non-test src)

| Site | Inside | Classification | Java |
|---|---|---|---|
| `actions/file_state_value.rs:188` `stat_if_found(Nofollow)`, `:201,230` `read_symbolic_link_unchecked`, `:281` `get_fast_digest`, `:284` `is_readable` (via `FileStateValue::create`, called at `file_state.rs:307`) | FILE_STATE | allowed (Java does the same) | `J:com/google/devtools/build/lib/actions/FileStateValue.java:147,189` |
| `directory_listing.rs:152` `readdir(Nofollow)` | DIRECTORY_LISTING_STATE | allowed (Java does the same) | `J:.../skyframe/DirectoryListingStateFunction.java:51` |
| `bzl_compile.rs:399-401` `get_input_stream` + `read_to_end` | BZL_COMPILE, after FILE dep (`bzl_compile.rs:382`) | allowed: content read after the FileValue dep; FILE_STATE's proxy/digest dirties it (Java does the same) | `J:.../skyframe/BzlCompileFunction.java:98,122-123` |
| `repo_file.rs:214-216` `get_input_stream` | REPO_FILE, after FILE dep (`repo_file.rs:187`) | allowed (Java does the same) | `J:.../skyframe/RepoFileFunction.java:88,112` |
| `ignored_subdirectories.rs:307-309` `get_input_stream` (`.bazelignore`) | IGNORED_SUBDIRECTORIES, after FILE dep (`:475,497`) | allowed (Java does the same) | `J:.../skyframe/IgnoredSubdirectoriesFunction.java:79-80,151,170` |
| `package_function.rs:1362-1364` `get_input_stream` (BUILD file) | PACKAGE, after FILE dep (`package_function.rs:1504`) | allowed (Java does the same) | `J:.../skyframe/PackageFunction.java:653,1293-1294` |
| `pkgcache.rs:155,170` `stat_if_found(Follow)` in `get_package_build_file_nullable`, reached from PACKAGE via `SkyframePackageLocator` `package_function.rs:141-150` (non-Skyframe globber subpackage check) | PACKAGE (legacy globbing) | allowed (Java does the same): Java's `GlobCache` asks `packageLocator.getBuildFileForPackage` (`J:com/google/devtools/build/lib/packages/GlobCache.java:154`), then PACKAGE declares the GLOBS dep afterwards (`package_function.rs:917` = `J:.../skyframe/PackageFunctionWithSingleGlobsDep.java:174`). The globber's own directory reads are in blaze_packages (not checked here) |
| `repos.rs:708-711` `.exists()` (`is_valid_repo_root`), `:722` `read_symbolic_link`, `:729` `delete`, `:738` `create_directory_and_parents`, `:740` `create_symbolic_link_to_fragment` (`ensure_symbolic_link`) | REPOSITORY_DIRECTORY through `OracleRepos::compute_repository_directory` -> `symlink_repo_root` `repos.rs:605,623` | allowed (Java does the same; REPOSITORY_DIRECTORY is NonHermetic, `RepositoryFetchFunction.symlinkRepoRoot` `J:com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:800`). `OracleRepos` is the W3 dev seam, used only by `crates/blaze_query/src/bin/blaze-query-dev.rs`; writes are G7 |
| `pkgcache.rs:92` `root_path.exists()` (`PathPackageLocator::create`) | outside any SkyFunction (`sync_package_loading`) | not relevant | `PathPackageLocator.create` at command start |
| `repos.rs:445` `std::fs::read` (`OracleRepos::load`) | outside (setup) | not relevant | none (dev seam) |
| `bundled_fs.rs:263-347` (`FileSystem` impl over embedded files) | it is a file system, not a reader | not relevant | embedded builtins |

Bypass count for this crate: **0** (no site where Java requests a FileValue/DirectoryListingValue and Rust reads directly).
All other grep hits (`globs.rs:364-490`, `package_lookup.rs:574,620,710`, `project_function.rs:153`, `recursive_pkg.rs:209,388`, `file.rs:290`, `directory_listing.rs:253`, `repos.rs:675`, `bzl_compile.rs:382-413`) call `exists()`/`is_file()`/`is_directory()`/`get_digest()` on a `FileValue` or `LocalRepositoryLookupValue` obtained as a dep, not on the file system.

### Greps run (cwd `crates/blaze_sky_loading/src`, test dirs filtered)

- `grep -rnE "impl\s*(<[^>]*>)?\s*(blaze_skyframe::)?Key\s+for" src` (32 hits, 18 in scope)
- `grep -rn "client::<\|\.client()"` (0)
- `grep -rnE "^\s*(pub(\([^)]*\))?\s+)?static\s|thread_local!|OnceLock|LazyLock|Mutex<|RwLock<|Atomic[A-Z]|RefCell|SystemTime|Instant::now|std::env::var|env::var\("`
- `grep -rnE "\.exists\(|\.stat\(|\.stat_if_found\(|\.stat_nullable\(|read_dir|readdir\(|get_directory_entries|read_symbolic_link|readlink|get_input_stream|read_content|std::fs::|fs::read|read_to_string|\.is_file\(|\.is_directory\(|\.is_dir\(|\.metadata\(|File::open|\.get_digest\(|get_fast_digest|\.is_symbolic_link\(|\.get_file_size\(|\.resolve_symbolic_links|\.create_directory|\.delete\(|\.create_symbolic_link|write_"`
- `grep -rnE "file_system_utils|read_content|read_all|is_readable|is_executable|is_writable|get_last_modified|stat_nullable|\.readdir\(|get_directory_entries|FileStateValue::create|resolve_symbolic_links|...|read_to_end"`
- `grep -rn "PACKAGE_PROGRESS\|KEEP_ALIVE\|bzl_compile_cache\|BUNDLED_FS\|show_loading_progress\|pkg_locator\.read\|deleted_packages\.read"`
- `grep -rn "OracleRepos\|symlink_repo_root" crates --include=*.rs`
- Java: `grep -n "class \|boolean equals\|NotComparable"` on each value class above; `grep -n "pkgLocator\|deletedPackages\|showLoadingProgress\|bzlCompileCache\|packageProgress"` in `SkyframeExecutor.java`/`SequencedSkyframeExecutor.java`; `grep -rn repoContentsCachePathSupplier`; `DirtyBuildingState.java:214`, `AbstractInMemoryMemoizingEvaluator.java:432`.

### Not checked

- The other 14 production keys of this crate (part 2): RecursivePkg, PackageError(Message), TransitiveTarget, Package, Globs, the 5 package-piece keys, RepoFile, RepoPackageArgs, TargetPattern.
- Equality of each Java type wrapped in `PrecomputedValue` against its Rust `PartialEq` (row 11 lists them, not verified).
- Key-side `Eq`/`Hash` consistency (only noted the manual ones).
- FS reads inside blaze_packages called from PACKAGE (non-Skyframe globber, `create_non_skyframe_globber`) and inside the `ExternalRepos` implementations in other crates.
- Sections D and E (not asked), except where a B row touches them (`bzl_compile_cache`, `PACKAGE_PROGRESS`, `KEEP_ALIVE`).

Helper draft: **I2 survey, group 1 part 2: `blaze_sky_loading` (14 keys) + sections D and E for the whole crate**

Tree: HEAD `4f92912` plus uncommitted work of others (line numbers as of 2026-11-19). Read-only.
Rust paths are under `crates/`; Java under `J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`.

Java pruning rule used throughout: `J:com/google/devtools/build/skyframe/DirtyBuildingState.java:214-218`
`unchangedFromLastBuild` = `!(newValue instanceof NotComparableSkyValue) && lastBuildValue.equals(newValue)`.
A Java `record` therefore prunes by component-wise `equals` (each component's own `equals`, which
is identity for classes without an override), unless the record implements a
`NotComparableSkyValue` sub-interface.

### A. Keys

All 14 Java function names are `createHermetic` except TARGET_PATTERN (`createSemiHermetic`). The new
`HERMETICITY`/`values_equal` items from I1 are not in the tree yet (`grep -n "HERMETICITY\|values_equal" crates/blaze_skyframe/src/*.rs`: none).

| # | Rust key (`file:line`) | FUNCTION | Rust value type (def) | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `blaze_sky_loading/src/package_function.rs:100` `PackageKey` | `PACKAGE` | `PackageValue` `package_function.rs:107` (`Arc<Package>`) | none | `PackageIdentifier` itself, `J:com/google/devtools/build/lib/cmdline/PackageIdentifier.java:249` `functionName()` = PACKAGE | `J:.../lib/skyframe/PackageValue.java:32` implements `PackageoidValue`, which `extends NotComparableSkyValue` (`J:.../lib/skyframe/PackageoidValue.java:21`) | HERMETIC `J:.../lib/skyframe/SkyFunctions.java:56` | `IDENTITY` (prelim.): NotComparable. |
| 2 | `package_pieces.rs:66` `PackagePieceForBuildFileKey` | `PACKAGE` (same name as row 1; correct) | `PackagePieceValueForBuildFile` `package_pieces.rs:100` | none | `PackagePieceIdentifier.ForBuildFile` itself, `J:.../lib/packages/PackagePieceIdentifier.java:51`; `functionName()` returns `SkyFunctions.PACKAGE` at `:69-71`. `PackageFunction.compute` takes either key (`J:.../lib/skyframe/PackageFunction.java:361-376`: "either a PackageIdentifier or a PackagePieceIdentifier.ForBuildFile") and returns `PackagePieceValue.ForBuildFile` for the latter | `J:.../lib/skyframe/PackagePieceValue.java:57` `record ForBuildFile` implements `PackagePieceValue` (`:32` `extends PackageoidValue` → NotComparable). Record `equals` exists but is never consulted by the evaluator | HERMETIC `SkyFunctions.java:56` (shared with row 1; `SkyFunctionName.java:60-64` `create` asserts one hermeticity per interned name, so rows 1 and 2 must declare the same `HERMETICITY`) | `IDENTITY` (prelim.): NotComparable via interface. Rust dispatch: one `.function(PACKAGE, ...)` that downcasts (`skyframe_executor.rs:377-394`), consistent with Java. |
| 3 | `package_pieces.rs:82` `EvalMacroKey` | `EVAL_MACRO` | `PackagePieceValueForMacro` `package_pieces.rs:121` | none | `PackagePieceIdentifier.ForMacro` `J:.../lib/packages/PackagePieceIdentifier.java:94`, `functionName` `:129-131` | `J:.../lib/skyframe/PackagePieceValue.java:83` `record ForMacro` implements `PackagePieceValue` → NotComparable | HERMETIC `SkyFunctions.java:62` | `IDENTITY` (prelim.): NotComparable. |
| 4 | `package_pieces.rs:151` `PackageDeclarationsKey` | `PACKAGE_DECLARATIONS` | `PackageDeclarationsValue` `package_pieces.rs:159` (`Arc<PackageMetadata>`, `Arc<PackageDeclarations>`, `Arc<StarlarkSemantics>`, `RepositoryMapping`) | none on the value; every component has one: `PackageMetadata` manual `blaze_packages/src/package/package.rs:72` (all record components), `PackageDeclarations` manual `package.rs:406` (compares `DeclarationsInner` = package_args, make_env, transitive_loads; ignores `mutable`, `contains_errors`, `failure_detail`), `StarlarkSemantics` derived `blaze_starlark/src/semantics.rs:39`, `RepositoryMapping` manual `blaze_cmdline/src/repository_mapping.rs:171` | `J:.../lib/skyframe/PackageDeclarationsValue.java:55` `record Key` | `J:.../lib/skyframe/PackageDeclarationsValue.java:39` `record` (SkyValue, not NotComparable). Class comment `:33-35`: "The purpose of this value is to allow change-pruning on the transitive dependency from a PackagePieceValue.ForMacro to its PackagePieceValue.ForBuildFile - since package pieces are not comparable." Components: `Package.Metadata` record (`J:.../lib/packages/Package.java:1389`; only `hashCode` overridden at `:1406`), `Declarations.equals` `Package.java:1572` (packageArgs, makeEnv, transitive loads), `StarlarkSemantics.equals` (`J:net/starlark/java/eval/StarlarkSemantics.java:224`), `RepositoryMapping.equals` (`J:.../lib/cmdline/RepositoryMapping.java:65`) | HERMETIC `SkyFunctions.java:57-58` | `EQ-WRITE` (prelim.): nothing blocks; derive or write `PartialEq` over the four fields (deref through `Arc`). Rust `IndexMap ==` for make_env is order-insensitive like `ImmutableMap.equals`. Note: the Rust `PackageDeclarations` also carries `contains_errors`/`failure_detail` (PORT, `package.rs:231-235`) that Java's `Declarations` does not have; equality correctly ignores them, and the only reader through this value (`package_pieces.rs:538`, `new_package_piece_for_macro_builder`) calls `copy_for_packageoid` (`package_piece.rs:612`), which resets them, so a pruned value cannot leak a stale error flag. This is the one key whose pruning the Java authors designed for: high value. |
| 5 | `package_pieces.rs:185` `MacroInstanceKey` | `MACRO_INSTANCE` | `MacroInstanceValue` `package_pieces.rs:192` (`Arc<MacroInstance>`) | none; `MacroInstance` (`blaze_packages/src/macros/macro_instance.rs:30`) has no `PartialEq` | `J:.../lib/skyframe/MacroInstanceValue.java:44` `record Key` | `J:.../lib/skyframe/MacroInstanceValue.java:37` `record(MacroInstance)`; `MacroInstance.equals` `J:.../lib/packages/MacroInstance.java:107` (attr values via `RuleOrMacroInstance.equalsHelper` `RuleOrMacroInstance.java:116`: prefix length, `Arrays.equals(attrValues)`, attrBytes, label; plus metadata, declarations, parent (recursive equals), generatorName, buildFileLocation, parentCallStack (`CallStack.java:102` equals), macroClass (no `equals` in `MacroClass.java`: identity), sameNameDepth) | HERMETIC `SkyFunctions.java:63-64` | `EQ-WRITE` (prelim.): needs `PartialEq` for `MacroInstance` = `AttributeContainer` (`blaze_packages/src/target/rule_or_macro_instance.rs:51`, no `PartialEq`; attr values may hold Starlark values, Java compares them with `equals`), `Arc<MacroClass>` by `Arc::ptr_eq` (Java identity), `parent` recursively, `CallStackNode` (derived, ok), `PackageMetadata`/`PackageDeclarations` (ok). Ignore `frozen_state` OnceLock (Java compares the compacted arrays; check that omitted-default compaction does not change the result). |
| 6 | `package_pieces.rs:210` `NonFinalizerPackagePiecesKey` | `NON_FINALIZER_PACKAGE_PIECES` | `NonFinalizerPackagePiecesValue` `package_pieces.rs:218` | none | `J:.../lib/skyframe/NonFinalizerPackagePiecesValue.java:106` `record Key` | `J:.../lib/skyframe/NonFinalizerPackagePiecesValue.java:45` `record` implements `PackagePieces, SkyValue` (not NotComparable). Components: `ImmutableMap<PackagePieceIdentifier, PackagePiece>` (PackagePiece has no `equals`: identity), `ImmutableList<PackagePieceIdentifier>`, `@Nullable EvalException` (no `equals` in `EvalException.java`: identity), `ImmutableSortedMap<String, Target>` (Rule/InputFile/OutputFile/PackageGroup have no `equals`: identity), `ImmutableSortedMap<String, MacroInstance>` (deep, row 5), semantics, mapping | HERMETIC `SkyFunctions.java:65-66` | `EQ-WRITE` (prelim.), mixed identity/value: pieces by `Arc::ptr_eq`; `Option<EvalError>` equal only if both `None` (a fresh evaluation never reuses the exception object); targets by identity (existing `Target::eq` `blaze_packages/src/target/target.rs:43` is identity except `OutputFile` and `FakeLoadTarget`, which compare by value: `OutputFile` stronger than Java, use `Arc::ptr_eq` for it); macro instances need row 5's `MacroInstance` eq; `error_keys` Vec ok. Pruning only fires when every piece node stayed clean, i.e. the value was recomputed for another dep. |
| 7 | `globs.rs:188` `GlobsKey` | `GLOBS` | `GlobsValue` `globs.rs:116` (`HashSet<PathFragment>`) | derived `globs.rs:115` | `J:.../lib/skyframe/GlobsValue.java:150` `Key` | `J:.../lib/skyframe/GlobsValue.java:33`, `equals` at `:48` (set equality of `matches`) | HERMETIC `SkyFunctions.java:55` | `EQ-OK` (prelim.): `HashSet ==` is set equality, as `ImmutableSet.equals`. |
| 8 | `recursive_pkg.rs:119` `RecursivePkgKey` | `RECURSIVE_PKG` (`recursive_pkg.rs:31`) | `RecursivePkgValue` `recursive_pkg.rs:129` (`packages: Vec<String>`, `has_errors`) | derived `recursive_pkg.rs:128` | `J:.../lib/skyframe/RecursivePkgValue.java:74` `Key extends RecursivePkgSkyKey` | `J:.../lib/skyframe/RecursivePkgValue.java:37`: no `equals` (identity); only the `EMPTY` singleton (`:39-40`, returned by `create` at `:51-55` for empty and error-free) is shared, so two empty results are equal in Java | HERMETIC `SkyFunctions.java:94` | `RUST-STRONGER` (prelim.): Java holds a `NestedSet<String>` (identity) and never prunes a non-empty result. Must not opt in. (An opt-in limited to "both empty and no errors" would match Java exactly; not worth it.) |
| 9 | `recursive_pkg.rs:588` `PackageErrorMessageKey` | `PACKAGE_ERROR_MESSAGE` (`recursive_pkg.rs:37`) | `PackageErrorMessageValue` enum `recursive_pkg.rs:596` | derived `recursive_pkg.rs:595` | `J:.../lib/skyframe/PackageErrorMessageValue.java:80` `Key` | `J:.../lib/skyframe/PackageErrorMessageValue.java:31` abstract; `NO_ERROR_VALUE`/`ERROR_VALUE` singletons (`:104-131`, identity = per-variant equality); `NoSuchPackageExceptionValue.equals` `:154` (message equality). Class comment `:28`: "a change-pruning-friendly convenience SkyValue" | HERMETIC `SkyFunctions.java:60-61` | `EQ-OK` (prelim.): derived enum equality = Java's three cases. |
| 10 | `recursive_pkg.rs:646` `PackageErrorKey` | `PACKAGE_ERROR` (`recursive_pkg.rs:34`) | `PackageErrorValue` unit struct `recursive_pkg.rs:644` | none | `J:.../lib/skyframe/PackageErrorFunction.java:50` `Key` | none: `PackageErrorFunction.compute` (`J:.../lib/skyframe/PackageErrorFunction.java:78-95`) always throws `PackageErrorFunctionException` (or `IllegalStateException`); no value class | HERMETIC `SkyFunctions.java:59` | `IDENTITY` (prelim.): moot (never a value). Rust `package_error_function` (`recursive_pkg.rs:655`) also never returns `Some`. |
| 11 | `repo_file.rs:95` `RepoFileKey` | `REPO_FILE` | `RepoFileValue` `repo_file.rs:44` (`Option<OwnedFrozenValue>`, `Vec<String>`) | none | `J:.../lib/skyframe/RepoFileValue.java:49` `Key` | `J:.../lib/skyframe/RepoFileValue.java:29` `record(ImmutableMap<String,Object> packageArgsMap, ImmutableList<String> ignoredDirectories)`; map values are Starlark objects with value `equals` (`StarlarkList.java:244`, `Dict.java:209`, `Tuple.java:129`, String, Boolean) | HERMETIC `SkyFunctions.java:153` | `EQ-WRITE` (prelim.): blocked by `OwnedFrozenValue` (needs Starlark structural equality, e.g. `Value::equals` on both frozen dicts, keyed order-insensitively like `ImmutableMap.equals`). Also normalise Rust `None` vs Java empty map (`RepoFileValue::empty()` `repo_file.rs:51` is `None`; a `repo()` call with no args may give `Some(empty dict)`: Java has one representation). |
| 12 | `repo_file.rs:377` `RepoPackageArgsKey` | `REPO_PACKAGE_ARGS` | `RepoPackageArgsValue` `repo_file.rs:324` (`PackageArgs`) | none (only `Clone, Debug` at `repo_file.rs:323`); `PackageArgs` derived `blaze_packages/src/package/package_args.rs:135` | `J:.../lib/skyframe/RepoPackageArgsFunction.java:85` `Key` | `J:.../lib/skyframe/RepoPackageArgsFunction.java:38` `RepoPackageArgsValue`, `equals` at `:57` (`packageArgs.equals`, `PackageArgs` is `@AutoValue` `J:.../lib/packages/PackageArgs.java:36`) | HERMETIC `SkyFunctions.java:151-152` | `EQ-WRITE` (prelim.): trivial, derive `PartialEq`. `PackageArgs` fields: `IndexSet<Label> ==` is set equality like `ImmutableSet`; `RuleVisibility` derived (Java `RuleVisibility.java:248` equals / `PackageGroupsRuleVisibility` AutoValue), `License` derived (Java `License.java:185`), `FeatureSet` derived: agree. |
| 13 | `transitive_target.rs:55` `TransitiveTargetKey` | `TRANSITIVE_TARGET` (`transitive_target.rs:30`) | `TransitiveTargetValue` `transitive_target.rs:95` (`Arc<LabelNestedSet>`, `Option<NoSuchTargetException>`, bool) | none | `J:.../lib/skyframe/TransitiveTargetKey.java:30` | `J:.../lib/skyframe/TransitiveTargetValue.java:34` sealed class, subclass `UnsuccessfulTransitiveTargetValue` `:54`; no `equals` anywhere in the file: identity | HERMETIC `J:.../lib/skyframe/TransitiveTargetKey.java:31` (`createHermetic("TRANSITIVE_TARGET")`) | `IDENTITY` (prelim.). |
| 14 | `target_pattern_function.rs:190` `TargetPatternKey` | `TARGET_PATTERN` | `TargetPatternValue` `target_pattern_function.rs:199` (`IndexSet<Label>`, `has_error`) | derived `target_pattern_function.rs:198` | `J:.../lib/skyframe/TargetPatternValue.java:195` `TargetPatternKey` | `J:.../lib/skyframe/TargetPatternValue.java:48` final class holding `ResolvedTargets<Label>`; no `equals` in either (`ResolvedTargets.java:31` has none): identity | SEMI_HERMETIC `SkyFunctions.java:68-69` | `RUST-STRONGER` (prelim.): must not opt in. |

Counts (14): EQ-OK 2 (GLOBS, PACKAGE_ERROR_MESSAGE); EQ-WRITE 5 (PACKAGE_DECLARATIONS, MACRO_INSTANCE,
NON_FINALIZER_PACKAGE_PIECES, REPO_FILE, REPO_PACKAGE_ARGS); IDENTITY 5 (PACKAGE ×2 keys, EVAL_MACRO,
PACKAGE_ERROR, TRANSITIVE_TARGET); RUST-STRONGER 2 (RECURSIVE_PKG, TARGET_PATTERN); MIXED 0 (row 6 is
EQ-WRITE with mixed identity/value fields).

Differences from the plan's 2.5 numbers: the plan counts `RepoPackageArgsValue` as EQ-DERIVABLE and
`RepoFileValue` as EQ-HARD: agreed. The plan does not mention the three package-piece records
(rows 4-6), which have Java value equality and no Rust `PartialEq`.

### D. Interior mutability reachable from the crate's value types (G4)

No value type defined in `blaze_sky_loading` has a `Mutex`/`RwLock`/`OnceLock`/`Atomic*`/`Cell`
field of its own (`grep -rnE "OnceLock|OnceCell|Mutex|RwLock|Atomic[A-Z]|RefCell|\bCell<" crates/blaze_sky_loading/src`:
hits are only in function/config structs, statics and tests). One and two levels down:

| Site | Reached from | What it is | Verdict |
|---|---|---|---|
| `blaze_packages/src/package/package.rs:228` `inner: RwLock<DeclarationsInner>` | `Package`, `PackagePiece`, `MacroInstance`, `Rule` (`package_declarations`), `PackageDeclarationsValue` | Java `Declarations.Builder` mutable fields. Every writer (`set_package_args` `:319`, `merge_package_args_from` `:325`, `set_make_environment` `:334`, `set_transitive_loads` `:341`) calls `check_mutable()` and panics after `build()` (`:356`). `build()` callers: `package_piece.rs:697`, `target_definition_context.rs:1108`, both inside `finish_build*` before the value exists | memo/builder state: fine (enforced by `check_mutable`) |
| `package.rs:230` `mutable: AtomicBool` | same | Java `instanceof Declarations.Builder`; flipped once by `build()` | fine |
| `package.rs:233` `contains_errors: AtomicBool`, `:235` `failure_detail: RwLock` | same | PORT: Java keeps these on the `Packageoid`, Rust on the declarations so targets can reach them. Writers (`set_contains_errors` `:385`, `set_failure_detail` `:394`, both `pub(crate)`, NOT guarded by `check_mutable`): `package_piece.rs:698-699` (BUILD piece finish), `:736-737` (macro piece finish, on its own copy from `copy_for_packageoid` `:612`), `target_definition_context.rs:1110-1111` (package finish; package-from-pieces uses its own copy, `package_piece.rs:779`), and `Rule::report_error` `blaze_packages/src/target/rule.rs:846` | **checked: no write after the value is returned.** `Rule::report_error` callers are all inside rule construction (`rule.rs:437, 485, 519`, `rule_class/rule_class.rs:619-713`, `attribute/attribute_provider.rs:204-486`, `attribute/computed_default.rs:331` = `StarlarkComputedDefaultTemplate.computePossibleValues` during `populateRuleAttributeValues`); Java documents the same restriction (`J:.../lib/packages/Rule.java:650-656`: "may only be called while the rule's package or package piece is being constructed"). No caller outside `blaze_packages`/`blaze_sky_loading` (`grep -rn "set_contains_errors\|set_failure_detail"`, `Arc::get_mut/make_mut` on packages: none). Recommendation: make the two setters panic when `!mutable` except during `finish_build*`, or move them to a builder-only path, so a later regression fails loudly; not needed for correctness today. |
| `blaze_packages/src/target/rule_or_macro_instance.rs:59` `frozen_state: OnceLock<FrozenAttrState>` | every `Rule`/`MacroInstance` in `Package`, `PackagePiece`, `NonFinalizerPackagePiecesValue`, `MacroInstanceValue` | Java `freeze()` compaction. Set by `freeze()` (`:91`); callers `package_piece.rs:661, 664`, `target_definition_context.rs:1072, 1077` (finish build) | memo set before return: fine |
| `rule_or_macro_instance.rs:34` `ImplicitTestsRef = Arc<RwLock<Vec<Label>>>` (field `implicit_tests` `:57`) | `test_suite` rules in packages/pieces | Java's shared `$implicit_tests` list from `TestSuiteImplicitTestsAccumulator`. Writers `blaze_packages/src/package/test_suite_implicit_tests_accumulator.rs:68, 97, 107`; the accumulator belongs to one builder (`target_definition_context.rs:99, 214`); `freeze()` snapshots the list into `frozen_state` (`rule_or_macro_instance.rs:108-111`) and reads after freezing use the snapshot (`:128-133`) | fine: a later write to the list (none found: each builder makes its own lists) would not be observed |
| `blaze_packages/src/builtins/env.rs:412` `GlobalsCache.cache: Mutex<HashMap<(usize, Vec<bool>), Arc<Globals>>>` | `StarlarkBuiltinsValue.globals_cache` (`starlark_builtins.rs:51`) | memo of `Globals` built from the value's own environments | memo: fine (dies with the value) |
| `blaze_packages/src/starlark_defs/rule_class_functions.rs:861-867` (`builder: Mutex<Option<..>>`, `rule_class`, `exported` OnceLocks), `macro_function.rs:65-72`, `aspect.rs:123, 127` (`exported`, `definition_cache: Mutex<HashMap<AspectParameters, ..>>`), `subrule.rs:47`, `provider.rs:169` (`key: OnceLock`), `value_cell.rs:43` | Starlark objects inside `BzlLoadValue.module: FrozenModule` (`bzl_load.rs:238`) | export state (set when the defining module exports the global) and keyed memos | export: write-once during BZL_LOAD of the defining (or, as in Java, the first exporting) module; Java mutates the same objects the same way (`StarlarkExportable.export`), so a re-run of a dependent `.bzl` sees `isExported()` and skips: same as Java. `definition_cache`: keyed memo, fine. Not followed further. |
| `blaze_cmdline/src/repository_mapping.rs:27` `inverse: Option<HashMap>` | `RepositoryMappingValue`, every value holding a `RepositoryMapping` | set at construction (`with_cached_inverse_map`), not interior-mutable | fine |

Answer to the plan's open question (08 §2.5 G4): nothing writes to a `Package` or its targets after
the PACKAGE (or PACKAGE piece) value is returned; the `RwLock`/`AtomicBool`s are builder state.

### E. Output, caches and side effects not through the graph (whole crate)

#### G5 direct output

| Site | What | Java | Verdict |
|---|---|---|---|
| — | `grep -rnE "eprintln!\|println!\|eprint!\|print!\|stderr\(\)\|stdout\(\)" crates/blaze_sky_loading/src` (non-test): **no hits** | | clean |
| `package_function.rs:1161` → `package_progress_receiver.rs:34` `static PACKAGE_PROGRESS` | `done_read_package` counts loaded packages for the UI; `completed_set` dedups by package id (PORT, module comment `:10-14`) | `J:.../lib/skyframe/PackageFunction.java:1259` `packageProgress.doneReadPackage(packageId)` (a side channel in Java too); reset per command in `SkyframeExecutor.preparePackageLoading` `J:.../lib/skyframe/SkyframeExecutor.java:1741-1742` | Java does the same, but **`PACKAGE_PROGRESS.reset()` has no caller** (`grep -rn "PACKAGE_PROGRESS.reset"`: none; compare `ANALYSIS_PROGRESS.reset()` at `blaze_sky_top/src/build_tool.rs:516`). In a server: counts accumulate, and because of `completed_set` a package re-loaded by a later command is never counted again. Needs: move to a field of the server-lifetime executor, call `reset()` in the `preparePackageLoading` port, and drop the dedup set once build settings load in the command's own graph (I2 or S). Also read by `blaze_sky_top/src/analysis_phase_runner.rs:364`, `blaze_buildtool/src/runtime/ui_state_tracker.rs:1750ff` |
| `package_function.rs:1088-1090` | `Loading package:` progress event via `env.listener()`, gated by `config.show_loading_progress` captured at registration (`skyframe_executor.rs:330`) | `PackageFunction.java:1081` area (`showLoadingProgress` field) | output path fine; the captured flag is G1 (part 1 / B) |
| Event sites `package_pieces.rs:582, 632`, `package_function.rs:438, 1258, 1271, 1785` | go to the builder's local event handler, replayed with `replay_on(env.listener())` (`package_pieces.rs:599`, `package_function.rs:682, 701, 840`) | same (Java `localEventHandler`) | fine, provided I1's event storage replays the PACKAGE node's events (the `Package.events` field `blaze_packages/src/package/package.rs:461` is replayed by PACKAGE itself, so a clean PACKAGE node in a later command will not replay them unless E8 stores them; same as Java's `ValueWithEvents`) |
| `transitive_target.rs:253`, `starlark_util.rs:40, 48, 144`, `bzl_compile.rs:567`, `bzl_load.rs:666`, `repo_file.rs:304` | `handler`/`reporter` parameters; callers pass `env.listener()` (`transitive_target.rs:298`, `bzl_compile.rs:452, 500`, `repo_file.rs:199-200`) | same | fine (spot-checked the callers listed) |
| `pkgcache.rs:84` | warning from `PathPackageLocator::create`, called from `sync_package_loading` with `NullEventHandler` (`skyframe_executor.rs:191`) | `BlazeWorkspace.getOrCreatePackageLocatorForCommand` also uses `NullEventHandler.INSTANCE` | outside functions; fine |
| `package_manager.rs:543, 594, 630` | pattern parsing errors to the caller's handler | `SkyframePackageManager`/`TargetPatternPhase` callers, outside Skyframe | outside functions; fine |

#### G6 caches outside the graph

| Site | What | Java | Verdict |
|---|---|---|---|
| `bzl_load.rs:775` `BzlLoadFunction.bzl_compile_cache: Mutex<HashMap<BzlCompileKey, BzlCompileValue>>`; read `:1020`, insert `:1032`, per-entry remove `:843` | inlined BZL_COMPILE results kept across Skyframe restarts | `J:.../lib/skyframe/SkyframeExecutor.java:393` `bzlCompileCache` (Caffeine, owned by the executor, passed to `BzlLoadFunction.create` at `:1015`); per-entry `invalidate` in `InliningAndCachingGetter.doneWithBzlCompileValue` `J:.../lib/skyframe/BzlLoadFunction.java:1520-1523` (= Rust `:843`); **`bzlCompileCache.invalidateAll()` at `SkyframeExecutor.java:1738` in `preparePackageLoading` (`:1693`), once per command**, with the comment "If the SkyFunctions never had a chance to restart (e.g. due to user interrupt, or an error in a --nokeep_going build), these may have stale entries." | Rust has no invalidate-all; harmless today only because `BzlLoadFunction` is built per evaluator. With a server-lifetime executor a stale entry left by an interrupted or `--nokeep_going` command would serve an old AST after the `.bzl` is edited (the cache hit requests the FILE dep, `:1035ff`, but returns the cached parse). Port: add `invalidate_all()` and call it from the `preparePackageLoading` port. The "inlining cache reset" asked about (`BzlLoadFunction.resetInliningCache` `BzlLoadFunction.java:374`) is a different thing: it resets `inlineCacheManager`, which is `null` for Bazel's `create(...)` (`:173-174`); its only callers at the pin are the class itself and `src/test/.../BuildViewTestCase.java` (`grep -rln "resetInliningCache\|createForInlining" src`). Not needed. |
| `bzl_load.rs:732` `static KEEP_ALIVE: Mutex<Vec<FrozenModule>>`, push `:1516` | pins every loaded module's heap | none (JVM GC) | not a lookup cache (never read); memory only. Grows by one module per BZL_LOAD run, including re-runs after edits. Plan 3.3 option (a), arena owned by the executor |
| `blaze_packages/src/bzl_context.rs:65` `BY_FILENAME`, write `:109` from `bzl_load.rs:1469` | module context by compiled file name, last writer wins | none | **Now write-only in production**: `of_filename` (`bzl_context.rs:126`) and `all_package_contexts` (`:115`) have no caller outside tests; `blaze_repo_rules/src/starlark/repository_module.rs:531, 603` now use `of_innermost_bzl` (`:648-652`), not the table (plan 3.1 is stale here). Memory only; can be deleted or moved to the arena |
| `blaze_packages/src/package/rule_factory.rs:214` `GENERATOR_NAME_HEAPS`, push `:220` | one frozen heap per `generator_name` allocation, forever | none | memory only (grows per package (re)load); arena |
| `package_progress_receiver.rs:34` `PACKAGE_PROGRESS` | see G5 | `SkyframeExecutor.java:397, 1741` | reset needed |
| `blaze_starlark/src/error.rs:360` `SOURCE_READER` (set by `blaze_runtime/src/runtime.rs:701`) | source lines for error stacks, read from the file system when an `EvalError` is converted inside a function (`error.rs:383`, comment `:375-379`) | Java reads at format time (`EvalException.formatCallStack`, `J:net/starlark/java/eval/EvalException.java:228`), outside the graph | belongs to section C (G2) of the owning group: an error message stored in an error value embeds file text read without a FILE dep; after an edit a clean error node would print the old line. Flag for `blaze_starlark`'s owner |
| `package_function.rs:165-169` `GlobsGlobber` Mutexes; `package_manager.rs:159` `value_lookup_lock` | per-call globber state; PackageManager lock outside Skyframe | same shapes in Java | fine |
| `bundled_fs.rs:376` `BUNDLED_FS`, `build_file_name.rs:23` `FRAGMENTS`, `precomputed.rs:124-157` key statics | constants | | fine |

#### G7 side effects

| Site | What | Java | Verdict |
|---|---|---|---|
| `repos.rs:605` → `symlink_repo_root` `:623` → `ensure_symbolic_link` `:719-741` (delete + create symlink, create parents) | `OracleRepos::compute_repository_directory` makes `<output_base>/external/<name>` a symlink | `J:.../lib/bazel/repository/RepositoryFetchFunction.java:800` `symlinkRepoRoot`; REPOSITORY_DIRECTORY is NONHERMETIC (`SkyFunctions.java:133-134`) | allowed (Java does the same, idempotent). `OracleRepos` is used only by tests and `blaze_query/src/bin/blaze-query-dev.rs:38`; production goes through the `ExternalRepos` impl in other crates |
| `bzl_load.rs:1516` KEEP_ALIVE push, `bzl_context.rs:109` register, `rule_factory.rs:220` heap push, `package_function.rs:1161` progress count | process-global writes from function bodies | — | repeatable (append/insert); memory and counter issues above |
| other | `grep -rnE "fs::(write\|create_dir\|remove\|rename)\|File::create\|OpenOptions\|Command::new\|create_directory\|write_all\|\.delete\(\|create_symbolic_link\|TcpStream"` over `crates/blaze_sky_loading/src`: only `repos.rs` above and `bundled_fs.rs:244-299` (the in-memory bundled FS's own trait methods) | | none else |

### Greps run

- `grep -nE "impl\s*(<[^>]*>)?\s*(blaze_skyframe::)?Key\s+for" crates/blaze_sky_loading/src/*.rs crates/blaze_sky_loading/src/actions/*.rs` (32 production impls; this part covers 14)
- Java: `grep -nE "class|interface|record|equals|hashCode|NotComparable|implements|extends" ` on each of `PackageValue, PackageoidValue, PackagePieceValue, PackageDeclarationsValue, MacroInstanceValue, NonFinalizerPackagePiecesValue, GlobsValue, RecursivePkgValue, PackageErrorMessageValue, PackageErrorFunction, RepoFileValue, RepoPackageArgsFunction, TransitiveTargetValue, TargetPatternValue`
- `grep -rn "NotComparableSkyValue" J:com/google/devtools/build/skyframe/*.java`
- `grep -n "boolean equals"` in `Package.java, MacroInstance.java, RuleOrMacroInstance.java, Rule.java, PackagePiece.java, Packageoid.java, InputFile.java, OutputFile.java, PackageGroup.java, EnvironmentGroup.java, MacroClass.java, CallStack.java, PackageArgs.java, RuleVisibility.java, License.java, StarlarkSemantics.java, EvalException.java, RepositoryMapping.java, StarlarkList.java, Dict.java, Tuple.java, NestedSet.java, ResolvedTargets.java`
- `grep -nE "<names>" J:.../lib/skyframe/SkyFunctions.java`; `TransitiveTargetKey.java:25-40`; `SkyFunctionName.java` create*
- `grep -rnE "OnceLock|OnceCell|Mutex|RwLock|Atomic[A-Z]|RefCell|\bCell<|LazyLock|thread_local|static "` over `crates/blaze_sky_loading/src`, `crates/blaze_packages/src/{package,target,macros,rule_class,starlark_defs,builtins,attribute,aspect,types}`
- `grep -rnE "\.(set_contains_errors|set_failure_detail|set_package_args|merge_package_args_from|set_make_environment|set_transitive_loads)\("` over `crates`; `grep -rn "copy_for_packageoid"`; `grep -rn "\.report_error("`; `grep -rn "\.freeze()"`; `grep -rnE "Arc::(get_mut|make_mut)\("`
- `grep -rnE "eprintln!|println!|eprint!|print!|stderr\(\)|stdout\(\)"`, `grep -rn "listener()\|\.handle(\|\.post("`, `grep -rniE "cache"` over `crates/blaze_sky_loading/src`
- `grep -rn "PACKAGE_PROGRESS\|package_progress"` over `crates`; Java `grep -n "packageProgress"` in `SkyframeExecutor.java`, `PackageFunction.java`
- `grep -n "bzlCompileCache\|resetInliningCache\|InliningAndCachingGetter"` in `BzlLoadFunction.java`; `grep -rln "resetInliningCache\|createForInlining" src` (whole Bazel tree)
- `grep -rn "BazelModuleContext::register\|of_filename\|all_package_contexts\|BY_FILENAME"` over `crates`
- `grep -rnE "fs::(write|create_dir|remove|rename)|File::create|OpenOptions|Command::new|create_directory|write_all|\.delete\(|create_symbolic_link|TcpStream"` over `crates/blaze_sky_loading/src`

### Not checked

- Whether `AttributeContainer` attr values (`AttrValue`) all have Java-compatible equality (needed for rows 5-6 EQ-WRITE); `MacroInstance` compaction (`frozen_state.omitted`) vs Java's compacted arrays in `equalsHelper`.
- `Package.posts: Vec<Arc<dyn Postable>>` (`package.rs:462`) and `PackagePiece.posts`: whether any `Postable` impl holds interior mutability.
- `LabelConverter.label_cache` (`blaze_packages/src/types/label_converter.rs:27`): whether a converter outlives a function call (if held in a captured struct it would be a G6 cache).
- `NonSkyframeGlobber`/`GlobCache` file-system reads (G2, section C, not asked here).
- Starlark heap-level mutability inside `FrozenModule` beyond the export `OnceLock`s listed.
- Part 1's 18 keys (section A) are in the other helper's file; section D/E above covers their value types and functions too (`FileStateValue`, `FileValue`, directory listings, lookups, `BzlLoadValue`, `BzlCompileValue`, `StarlarkBuiltinsValue`, `PrecomputedValue` (`Arc<dyn Any>`, contents not followed), `ProjectValue`, `RepositoryMappingValue`, `RepositoryDirectoryValue`, `IgnoredSubdirectoriesValue`, `LocalRepositoryLookupValue`, `EmptySkyValue`): none has an interior-mutable field one level down besides those listed.

### F. Group 1 worker: files, order, hand-back

Files (all `crates/blaze_sky_loading/src/` unless a full path is given): `file_state.rs`, `file.rs`,
`directory_listing.rs`, `file_chain_uniqueness.rs`, `package_lookup.rs`, `containing_package_lookup.rs`,
`local_repository_lookup.rs`, `ignored_subdirectories.rs`, `precomputed.rs`, `repos.rs`,
`project_function.rs`, `starlark_builtins.rs`, `bzl_compile.rs`, `bzl_load.rs`, `package_function.rs`,
`package_pieces.rs`, `globs.rs`, `recursive_pkg.rs`, `repo_file.rs`, `transitive_target.rs`,
`target_pattern_function.rs`, `sky_functions.rs` (hermeticity next to the names),
`skyframe_executor.rs` (cells, resets), `package_progress_receiver.rs`. Cross-crate, assigned to
this group: `crates/blaze_packages/src/package/package.rs` (guard `set_contains_errors` and
`set_failure_detail`), `crates/blaze_packages/src/macros/macro_instance.rs` and
`crates/blaze_packages/src/target/rule_or_macro_instance.rs` (`PartialEq` for
`MacroInstance`/`AttributeContainer`), `crates/blaze_packages/src/target/target.rs` (only if
`OutputFile` must compare by identity inside `NonFinalizerPackagePiecesValue`). If root accepts
cross-group item X1, the group also owns the new shared precomputed module in `crates/blaze_skyframe`.

Order:
1. X1: the shared `PrecomputedKey`/`PrecomputedValue` with a stored equality function. This comes
   first because groups 2, 3, 4 and 5 need it.
2. `HERMETICITY` on all 32 keys. `PackageKey` and `PackagePieceForBuildFileKey` share `PACKAGE` and
   must declare the same value (`J:com/google/devtools/build/skyframe/SkyFunctionName.java:60-64`).
3. Opt in the 11 EQ-OK keys.
4. Write equality for the 7 EQ-WRITE and 2 MIXED keys:
   - `LocalRepositoryLookupValue::Successful` compares the name only;
   - `PackageLookupValue::NoRepository` is never equal;
   - `ContainingPackageLookupValue::NoContainingPackage` with a reason is never equal;
   - `PackageDeclarationsValue`, `RepoPackageArgsValue`;
   - `MacroInstanceValue`, `NonFinalizerPackagePiecesValue`;
   - `RepoFileValue` (Starlark structural equality).
5. Leave the 4 RUST-STRONGER keys and the 8 IDENTITY keys at the default, each with a comment
   quoting the Java.
6. G1 cells: `show_loading_progress` becomes a shared `Arc<AtomicBool>`; `repo_contents_cache_path`
   becomes a supplier; `pkg_locator` and `deleted_packages` keep their cells. The invalidations
   (`onPkgLocatorChange`, `setDeletedPackages` deletion) are I3's. Leave a marked hook.
7. G6 resets:
   - `bzl_compile_cache.invalidate_all()` per command (`J:.../skyframe/SkyframeExecutor.java:1738`);
   - `PACKAGE_PROGRESS.reset()` per command (`:1741-1742`), moving it off the static.

Hand back:
- Per key, one line: Java `equals` location, the Rust opt-in (`values_equal` body or default), and the
  final verdict.
- Per captured field: its class (a/b/c/d) and what was done.
- Tests: the Java tests of each value's `equals` that exist (`FileStateValueTest`,
  `DirectoryListingStateValueTest`, `PackageLookupValueTest`, `PrecomputedValueTest`; check the names
  under `src/test/java`), plus one test per hand-written `values_equal` covering each variant.

## Group 2: blaze_bzlmod + blaze_repo_rules (19 keys)

Helper draft: **I2 survey, group 2: `blaze_bzlmod` + `blaze_repo_rules`**

Tree: HEAD `4f92912` plus uncommitted work (line numbers as of 2026-11-19). Read-only survey; no Cargo.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`. `S:` = `J:com/google/devtools/build/lib/skyframe/SkyFunctions.java`.
`BZ:` = `J:com/google/devtools/build/lib/bazel/bzlmod/`. `M:` = `crates/blaze_bzlmod_model/src/`.

### A. Keys and value equality (19 production keys: blaze_bzlmod 13, blaze_repo_rules 6)

Notes used by several rows:
- Guava `ImmutableMap`/`ImmutableBiMap`/`ImmutableSet` equality is order-insensitive; so is `IndexMap`/`IndexSet` `PartialEq`. `ImmutableList` ↔ `Vec` both ordered. AutoValue compares `byte[]` with `Arrays.equals` (Rust `Vec<u8>` ==). Checked `Table` (`M:dep_graph.rs:55`, cell-set equality = `AbstractTable.equals`), `AttributeValues` (`M:repo_spec.rs:83`, `Map.equals` of the Dict), `Checksum` (`M:checksum.rs:89`, ignores `use_subresource_integrity` like `Checksum.java:139`), `Event` (`crates/blaze_events/src/event.rs:465` = `Event.java:272`), `RepositoryMapping` (`crates/blaze_cmdline/src/repository_mapping.rs:171` = `RepositoryMapping.java:65`, entries + contextRepo).
- **InterimModule registry (new finding).** Java `InterimModule` is AutoValue including `getRegistry()` (`BZ:InterimModule.java:109`); `Registry extends NotComparableSkyValue` (`BZ:Registry.java:27`) and `IndexRegistry` has no `equals` (`BZ:IndexRegistry.java:64`) → identity. Rust `InterimModule::eq` (`M:module.rs:192-212`) compares `registry.get_url()`, with the comment "Skyframe holds one instance per URL, so comparing URLs is equivalent". That holds within one evaluation only. With a kept graph, REGISTRY (non-hermetic, deps on the lockfile value, `LOCKFILE_MODE`, `VENDOR_DIRECTORY`, `MODULE_MIRRORS`, and `LAST_INVALIDATION` in REFRESH mode, `BZ:RegistryFunction.java:65-85`) re-runs and makes a new registry; Java then never prunes a `NonRootModuleFileValue`/`BazelModuleResolutionValue` built over it, Rust would prune and keep the old `Arc<dyn Registry>` (stale `known_file_hashes`, stale `bazel_registry.json` memo) inside the kept value. Fix before opting in: compare `registry` with `Arc::ptr_eq` (Java identity), or accept the deviation by decision.
- **Exceptions are identity in Java.** `ExternalDepsException extends Exception`, no `equals` (`BZ:ExternalDepsException.java`, grep `equals` empty). A record holding `ImmutableList<ExternalDepsException>` is equal to a recomputed one only when both lists are empty.

| # | Rust key (`file:line`) | FUNCTION | Rust value type (def) | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `crates/blaze_bzlmod/src/skyframe/keys.rs:72` ModuleFileKey | MODULE_FILE | `ModuleFileValue` enum `keys.rs:82` (Root `M:discovery.rs:29`, NonRoot `M:discovery.rs:18`) | derived `keys.rs:80`; fields derived (`M:discovery.rs:17,28`); `InterimModule` manual `M:module.rs:192` | `BZ:ModuleFileValue.java:88` record Key | `BZ:ModuleFileValue.java:31` interface; `NonRootModuleFileValue` record :50, `RootModuleFileValue` record :68 (registryFileHashes() override, not a component) | NonHermetic `S:149-150` | **MIXED** (prelim.): records compare all components and Rust matches, except `InterimModule.registry`: Java identity, Rust URL (see note). Root module has no registry, so the Root variant is EQ-OK; NonRoot is RUST-STRONGER until `registry` uses `Arc::ptr_eq`. |
| 2 | `keys.rs:132` BazelModuleResolutionKey | BAZEL_MODULE_RESOLUTION | `BazelModuleResolutionValue` `M:resolution.rs:51` | derived `M:resolution.rs:50` | `BZ:BazelModuleResolutionValue.java:35` lambda KEY | `BZ:BazelModuleResolutionValue.java:32` AutoValue (resolvedDepGraph, unprunedDepGraph `ImmutableMap<ModuleKey, InterimModule>`, registryFileHashes, selectedYankedVersions) | Hermetic `S:159-160` | **MIXED** (prelim.): same `InterimModule.registry` issue through `unpruned_dep_graph`; the other fields agree. `Module` (`M:module.rs:474`, derived) has no registry, `repo_spec` = Java `Module.getRepoSpec` (`BZ:Module.java:75`). |
| 3 | `keys.rs:148` BazelDepGraphKey | BAZEL_DEP_GRAPH | `BazelDepGraphValue` `M:dep_graph.rs:226` | derived `M:dep_graph.rs:225` (Table manual `M:dep_graph.rs:55`) | `BZ:BazelDepGraphValue.java:37` lambda | `BZ:BazelDepGraphValue.java:36` AutoValue (Java comment: table row order not in equality, tracked via abridged modules order) | Hermetic `S:169-170` | **EQ-OK** (prelim.): all 6 components compared with matching semantics; `canonical_repo_name_lookup` IndexMap = BiMap map equality. |
| 4 | `keys.rs:164` BazelModuleInspectorKey | BAZEL_MODULE_INSPECTION | `BazelModuleInspectorValue` `crates/blaze_bzlmod/src/inspector.rs:332` | none (`inspector.rs:331` derives Debug, Clone); `AugmentedModule` derived `inspector.rs:72` | `BZ:BazelModuleInspectorValue.java:69` lambda | `BZ:BazelModuleInspectorValue.java:53` record (depGraph, modulesIndex, extensionToRepoInternalNames `ImmutableSetMultimap`, moduleKeyToCanonicalNames, errors `ImmutableList<ExternalDepsException>`) | Hermetic `S:161-162` | **EQ-WRITE** (prelim.): blocked by `errors: Vec<ExternalDepsException>` (`M:external_deps_exception.rs:58`, no `PartialEq`, holds `Arc<dyn Error>`); Java compares exceptions by identity, so write: other fields `==` and both `errors` empty (or element-wise identity if errors become `Arc`). SetMultimap ↔ `IndexMap<_, IndexSet<_>>` equality agrees. |
| 5 | `keys.rs:200` BazelLockFileKey (KEY / HIDDEN_KEY) | BAZEL_LOCK_FILE | `BazelLockFileValue` `M:lockfile.rs:198` | derived `M:lockfile.rs:197` (PartialEq only: `Facts` holds `f64`) | `BZ:BazelLockFileValue.java:66` KEY, `:96` HIDDEN_KEY | `BZ:BazelLockFileValue.java:44` AutoValue (version, registryFileHashes, selectedYankedVersions, moduleExtensions, facts, factsVersions) | Hermetic `S:171-172` | **EQ-OK** (prelim.): fields match; `LockFileModuleExtension` (`M:lockfile.rs:170`) byte arrays as `Vec<u8>`. Caveat: `Facts` (`M:facts.rs:27`) is a `Vec` (ordered) where Java is `Dict` (`Map.equals`, unordered), harmless because Java and Rust both sort dict keys on normalisation (`BZ:Facts.java:91,113`); `FactValue::Float(f64)` NaN ≠ NaN in Rust (Java `StarlarkFloat.equals` not checked). |
| 6 | `keys.rs:229` RegistryKey | REGISTRY | `Arc<dyn Registry>` (trait `M:registry.rs:210`) | none (trait object) | `BZ:RegistryKey.java:27` record | `BZ:Registry.java:27` `interface Registry extends NotComparableSkyValue` | NonHermetic `S:175` | **IDENTITY** (prelim.): NotComparable; keep default. |
| 7 | `keys.rs:271` RepoSpecKey | REPO_SPEC | `RepoSpecValue` `keys.rs:279` | derived `keys.rs:278` | `BZ:RepoSpecKey.java:28` record | `BZ:RepoSpecValue.java:28` record (repoSpec, registryFileHashes); `RepoSpec` record `BZ:RepoSpec.java:29` | NonHermetic `S:176` | **EQ-OK** (prelim.): `RepoSpec` derived over manual `AttributeValues` (Dict map equality). |
| 8 | `keys.rs:325` YankedVersionsKey | YANKED_VERSIONS | `YankedVersionsValue` `M:registry.rs:66` | derived `M:registry.rs:65`; `Version` manual `M:version.rs:297` | `BZ:YankedVersionsValue.java:44` record Key | `BZ:YankedVersionsValue.java:29` record (`Optional<ImmutableMap<Version,String>>`); `Version.equals` `BZ:Version.java:199` | NonHermetic `S:177-178` | **EQ-OK** (prelim.) (Version eq body vs Java not re-read). |
| 9 | `keys.rs:417` SingleExtensionKey | SINGLE_EXTENSION | `SingleExtensionValue` `keys.rs:367` | derived `keys.rs:366`; `RootModuleFileFixup` derived `keys.rs:335` | `BZ:SingleExtensionValue.java:90` Key | `BZ:SingleExtensionValue.java:48` record (generatedRepoSpecs, BiMap, Optional WithFactors, Optional RootModuleFileFixup, Facts, factsVersion); `RootModuleFileFixup` record `BZ:RootModuleFileFixup.java:33` (`ImmutableListMultimap`, usage, Event) | Hermetic `S:165-166` | **EQ-OK** (prelim.): ListMultimap equality = per-key value lists ↔ `IndexMap<PathFragment, Vec<String>>`; Event eq ported; Facts caveat as row 5. |
| 10 | `keys.rs:440` SingleExtensionEvalKey | SINGLE_EXTENSION_EVAL | `SingleExtensionValue` `keys.rs:367` | derived `keys.rs:366` | `BZ:SingleExtensionValue.java:123` EvalKey | same as row 9 | NonHermetic `S:167-168` | **EQ-OK** (prelim.): as row 9. |
| 11 | `keys.rs:490` BazelModTidyKey | BAZEL_MOD_TIDY | `BazelModTidyValue` `keys.rs:448` | none (`keys.rs:447` Debug, Clone) | `BZ:BazelModTidyValue.java:51` lambda | `BZ:BazelModTidyValue.java:38` record (fixups, `Path buildozer`, moduleFilePaths, errors `ImmutableList<ExternalDepsException>`) | Hermetic `S:157-158` | **EQ-WRITE** (prelim.): blocked by `errors` (as row 4); other fields have `PartialEq` (`blaze_vfs::Path` manual `crates/blaze_vfs/src/path.rs:519`, Java `Path.equals` `J:com/google/devtools/build/lib/vfs/Path.java:218`). |
| 12 | `crates/blaze_bzlmod/src/skyframe/precomputed.rs:70` PrecomputedKey | PRECOMPUTED | `PrecomputedValue(Arc<dyn Any + Send + Sync>)` `precomputed.rs:36` | none (`precomputed.rs:35` Clone only) | `J:com/google/devtools/build/lib/skyframe/PrecomputedValue.java:191` Key | `J:.../skyframe/PrecomputedValue.java:45`, `equals` at :128 (`value.equals(other.value)`) | NonHermetic `S:22-23` | **EQ-WRITE** (prelim.): blocked by `Arc<dyn Any>`; needs an equality fn stored by `Precomputed::new`/`set` (all 13 `T`s here have `PartialEq`: `StarlarkSemantics` not checked). Second `PrecomputedKey` type exists in `crates/blaze_sky_loading/src/precomputed.rs` (same FUNCTION name, two graph nodes for `starlark_semantics`). |
| 13 | `precomputed.rs:213` ClientEnvironmentKey | CLIENT_ENVIRONMENT_VARIABLE | `EnvironmentVariableValue(Option<String>)` `precomputed.rs:224` | derived `precomputed.rs:223` | `J:.../skyframe/ClientEnvironmentFunction.java:37` Key | `J:.../skyframe/EnvironmentVariableValue.java:28` record (`@Nullable String value`) | NonHermetic `S:24-25` | **EQ-OK** (prelim.) |
| 14 | `crates/blaze_repo_rules/src/bzlmod/single_extension_usages.rs:167` SingleExtensionUsagesKey | SINGLE_EXTENSION_USAGES | `SingleExtensionUsagesValue` `single_extension_usages.rs:42` | derived (`:41` area, `Debug, Clone, PartialEq`) | `BZ:SingleExtensionUsagesValue.java:106` Key | `BZ:SingleExtensionUsagesValue.java:43` AutoValue (5 components; Java comment: extensionUsages order not in equality) | Hermetic `S:163-164` | **EQ-OK** (prelim.): fields and semantics agree (`RepositoryMapping` eq checked). |
| 15 | `crates/blaze_repo_rules/src/repository/repo_environment_function.rs:56` RepoEnvironmentKey | REPOSITORY_ENVIRONMENT_VARIABLE | `EnvironmentVariableValue` (blaze_bzlmod `precomputed.rs:224`) | derived `precomputed.rs:223` | `J:.../skyframe/RepoEnvironmentFunction.java:66` Key (functionName `REPOSITORY_ENVIRONMENT_VARIABLE`) | `RepoEnvironmentFunction.compute` returns `new EnvironmentVariableValue(repoEnv.get().get(name))` → record `EnvironmentVariableValue.java:28` | NonHermetic `S:30-31` (comment `S:29`: "like CLIENT_ENVIRONMENT_VARIABLE (see SkyframeExecutor#handleRepositoryEnvironmentChanges)") | **EQ-OK** (prelim.) |
| 16 | `crates/blaze_repo_rules/src/repository/repo_definition.rs:229` RepoDefinitionKey | REPO_DEFINITION | `RepoDefinitionValue` enum `repo_definition.rs:187` | manual `repo_definition.rs:196` (NotFound==NotFound; RepoOverride by path; Found by `Arc::ptr_eq`) | `J:com/google/devtools/build/lib/bazel/repository/RepoDefinitionValue.java:72` Key | `RepoDefinitionValue.java:49` `sealed interface RepoDefinitionValue extends NotComparableSkyValue` (records NotFound :56, RepoOverride :60, Found :64) | Hermetic `RepoDefinitionValue.java:50` (Rust comment `repo_definition.rs:34` cites `SkyFunctions.java` without line; the creation site is `RepoDefinitionValue.java:50`; Rust Key comment cites `:58`, actual `:72`, S4) | **RUST-STRONGER** (prelim.): must not opt in. Java `RepoDefinitionValue.java:29-47`: "This has to be a NotComparableSkyValue for a very subtle reason ... the repo mapping applicable to the .bzl file is *not* encoded in the RepoRule object ... we're relying on the repo marker files to be the ultimate "change pruners" of this SkyValue." Rust `==` returns true for NotFound/RepoOverride. |
| 17 | `crates/blaze_repo_rules/src/bzlmod/repo_mapping_entries.rs:77` ModuleExtensionRepoMappingEntriesKey | MODULE_EXTENSION_REPO_MAPPING_ENTRIES | `ModuleExtensionRepoMappingEntriesValue` `repo_mapping_entries.rs:32` | derived (`:31`) | `BZ:ModuleExtensionRepoMappingEntriesValue.java:55` Key | `BZ:ModuleExtensionRepoMappingEntriesValue.java:32` record (entries, `int entriesHashCode`, moduleKey) | Hermetic `S:180-181` | **EQ-OK** (prelim.): Rust drops `entriesHashCode` (PORT note `repo_mapping_entries.rs:13-16`); it is a function of `entries`, so equality agrees. |
| 18 | `crates/blaze_repo_rules/src/repository/directory_tree_digest.rs:80` DirectoryTreeDigestKey | DIRECTORY_TREE_DIGEST | `DirectoryTreeDigestValue` `directory_tree_digest.rs:33` | derived `:32` | `J:.../skyframe/DirectoryTreeDigestValue.java:42` Key | `J:.../skyframe/DirectoryTreeDigestValue.java:28` record (hexDigest) | Hermetic `S:36-37` | **EQ-OK** (prelim.) |
| 19 | `crates/blaze_repo_rules/src/bzlmod/bazel_fetch_all_function.rs:55` BazelFetchAllKey | BAZEL_FETCH_ALL | `BazelFetchAllValue` `bazel_fetch_all_function.rs:26` | derived (`:25`) | `BZ:BazelFetchAllValue.java:51` Key | `BZ:BazelFetchAllValue.java:34` record (`ImmutableList<RepositoryName>`) | Hermetic `S:173-174` | **EQ-OK** (prelim.) |

Counts: EQ-OK 12, EQ-WRITE 3 (rows 4, 11, 12), IDENTITY 1 (6), RUST-STRONGER 1 (16), MIXED 2 (1, 2). Plan 2.5 counted rows 1-2 as OK; the registry URL comparison makes them MIXED.
No Rust key sets `HERMETICITY` yet; every FUNCTION above has a Java creation site (none plain `create`).

### B. Inputs that are not dependencies (G1)

Registration: `crates/blaze_bzlmod/src/skyframe/mod.rs:119-174` (`register_bzlmod_functions(builder, Arc<BzlmodConfig>)`, closures capture `config` at :143-172; production caller `crates/blaze_sky_top/src/skyframe_executor.rs:317`, also `crates/blaze_repo_rules/src/setup.rs:151`), `crates/blaze_repo_rules/src/registration.rs:112-148` (`register_repo_rules_functions(builder, Arc<RepoServices>)`). Precomputed values are injected once at builder time (`mod.rs:123-136` via `Precomputed::set`; `registration.rs:84-94` `inject_repo_precomputed`); with a kept graph they must be re-injected per command with `Precomputed::set_on` (`precomputed.rs:132`), which exists but has no production caller (not grepped exhaustively). No `env.client::<T>()` reader in either crate (grep `client::<` / `.client()`: no hits).

#### `BzlmodConfig` (`crates/blaze_bzlmod/src/skyframe/config.rs:105-152`, built by `from_repository_options` :234-377)

| Field (`config.rs` line) | Read inside a function at | Class | Java |
|---|---|---|---|
| `workspace_root` :107, `output_base` :109, `install_base` :111 | `module_file_function.rs:225`, `registry_functions.rs:45`, `lock_file` via `mod.rs:158`, patch tmp dir `module_file_function.rs:1238` | (a) | `BlazeDirectories`, ctor args `J:com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:229-235, 239-240, 254-255` |
| `starlark_semantics` :113 | not read by functions; injected as precomputed `mod.rs:124` | (b) `PrecomputedValue.STARLARK_SEMANTICS` (`J:.../skyframe/PrecomputedValue.java:85`) | already a dep; needs per-command re-injection |
| `registries` :115, `module_mirrors` :117, `ignore_dev_deps` :119, `module_overrides` :121, `injected_repositories` :123, `check_direct_dependencies` :125, `bazel_compatibility_mode` :127, `lockfile_mode` :129, `allowed_yanked_versions` :131, `vendor_directory` :133 | only via `Precomputed` (`precomputed.rs:150-190`), injected `mod.rs:125-135` | (b) `ModuleFileFunction.REGISTRIES/IGNORE_DEV_DEPS/MODULE_OVERRIDES/INJECTED_REPOSITORIES` (`BZ:ModuleFileFunction.java:106-113`), `RegistryFunction.MODULE_MIRRORS` (`BZ:RegistryFunction.java:45`), `BazelModuleResolutionFunction.CHECK_DIRECT_DEPENDENCIES/BAZEL_COMPATIBILITY_MODE` (:64,66), `BazelLockFileFunction.LOCKFILE_MODE` (:51), `YankedVersionsUtil.ALLOWED_YANKED_VERSIONS` (:28), `RepositoryDirectoryValue.VENDOR_DIRECTORY`; all injected in `BazelRepositoryModule.getPrecomputedValues` (`BazelRepositoryModule.java:745-778`) | already deps; per-command re-injection needed |
| `last_registry_invalidation` :135 = `SystemTime::now()` :359 | via `LAST_INVALIDATION` in REFRESH mode, `registry_functions.rs:31-32` | (b) `RegistryFunction.LAST_INVALIDATION` (`BZ:RegistryFunction.java:42`), **but the value is wrong for a server**: Java keeps module field `lastRegistryInvalidation` (`BazelRepositoryModule.java:156`, init `Instant.EPOCH`), restores it from the graph in `beforeCommand` (:668-675, `getExistingValue(LAST_INVALIDATION.getKey())`), and advances it only after `INVALIDATION_INTERVAL` = 1 h (`BZ:RegistryFunction.java:51`; `BazelRepositoryModule.java:746-749`). Rust `now()` every command would invalidate every REGISTRY in each REFRESH command. | port the module field + interval rule |
| `client_env` :137 | captured by the CLIENT_ENVIRONMENT_VARIABLE closure `precomputed.rs:234-243` (registered `mod.rs:141`); read by `module_file_function.rs:93`, `resolution_function.rs:155` | (c) `SkyframeExecutor.clientEnv` `AtomicReference` (`J:.../skyframe/SkyframeExecutor.java:413`, function `:775`), invalidated per variable by `handleClientEnvironmentChanges` (:3874-3878) → `injectEnvironmentValues` (:3893-) | port the per-variable injection/invalidation; the closure must read a cell, not a snapshot |
| `registry_fetcher` :139 (wraps the command's `DownloadManager`, `crates/blaze_bzlmod/src/registry_fetcher.rs:20-30`) | `module_file_function.rs:1084`, `registry_functions.rs:89, 127` | (c) per-command module state without graph invalidation: Java `moduleFileFunction/repoSpecFunction/yankedVersionsFunction.setDownloadManager(downloadManager)` in `beforeCommand` (`BazelRepositoryModule.java:276-288`). The manager holds the command reporter (G5, same in Java). | becomes a settable cell (Java setter), reset per command |
| `registry_factory` :141 (`RegistryFactoryImpl` over a closure that snapshots `nonstrict_repo_env`, `config.rs:332-336`) | `registry_functions.rs:41-50` | (c) Java `RegistryFactoryImpl(nonstrictRepoEnvSupplier)` (`BazelRepositoryModule.java:167-168`), supplier set per command `:290`; read at `createRegistry` time (`BZ:RegistryFactoryImpl.java:78`). No invalidation in Java when it changes. | closure must read the per-command supplier, not a snapshot |
| `bazel_version` :143 | `resolution_function.rs:255` | (a) `BlazeVersionInfo` constant | stays |
| `builtin_modules` :145 | `module_file_function.rs:155, 321` | (a) `ModuleFileFunction` ctor arg (`BazelRepositoryModule.java:225-232`) | stays |
| `repository_directory_lookup` :147, `package_lookup` :151 | `module_file_function.rs:428-442, 745, 1011, 1179` | (a) stateless seams; production replaces both with Skyframe lookups (`crates/blaze_repo_rules/src/setup.rs:554-555`, impls :585, :623). The `Builtin*` stand-ins (`config.rs:52`, `module_file_function.rs:645`) are only used by `src/bin/blaze-mod-dev.rs` and tests. | stays |

#### `RepoServices` (`crates/blaze_repo_rules/src/services.rs:18-43`, built `setup.rs:526-536`)

| Field | Read inside a function at | Class | Java |
|---|---|---|---|
| `directories` :20 | `repository_fetch_function.rs:379` etc., `single_extension_eval_function.rs:114` | (a) | `BlazeDirectories` ctor arg (`BazelRepositoryModule.java:219-226`) |
| `repo_env` :23 | (1) REPOSITORY_ENVIRONMENT_VARIABLE closure `registration.rs:120-123` → `repo_environment_function.rs:62-70`; (2) repository_ctx/module_ctx (`repository_fetch_function.rs:842`, `runnable_extension.rs:480`, `base_external_context.rs:609` PATH, `starlark/os.rs:81`) | (1) (c) `SkyframeExecutor.repoEnv` `AtomicReference` (`SkyframeExecutor.java:414`, function :777) + per-variable injection `handleRepositoryEnvironmentChanges` (:3884-3888); Rust PORT note `repo_environment_function.rs:10-12` says this is skipped. (2) (c) `repoEnvSupplier` `MutableSupplier` set in `beforeCommand` (`BazelRepositoryModule.java:137, 289`), no invalidation (reads are recorded as REPOSITORY_ENVIRONMENT_VARIABLE deps via recorded inputs) | port both cells |
| `nonstrict_repo_env` :25 | `base_external_context.rs:543`, fetch/eval ctor `repository_fetch_function.rs:843`, `runnable_extension.rs:481` | (c) `nonstrictRepoEnvSupplier` (`BazelRepositoryModule.java:140, 290`) | cell |
| `download_manager` :27 | `base_external_context.rs:539`, `repository_fetch_function.rs:844`, `runnable_extension.rs:482` | (c) built per command in `beforeCommand` with `env.getReporter()` (`BazelRepositoryModule.java:276-288`), `setDownloadManager` | cell, reset per command |
| `timeout_scaling` :29 | `base_external_context.rs:2163` | (c) `setTimeoutScaling` in `beforeCommand` (`BazelRepositoryModule.java:311-320`) | cell |
| `process_wrapper` :31, `process_wrapper_kill_delay` :36, `process_wrapper_graceful_sigterm` :41 | `base_external_context.rs:2164-2168` | (c) `ProcessWrapper.fromCommandEnvironment(env)` + `setProcessWrapper` (`BazelRepositoryModule.java:292-295`) | cell |
| `repo_contents_cache` :43 | `repository_fetch_function.rs:482, 565` | (a) object of server lifetime (`RepositoryCache` field `BazelRepositoryModule.java:136`) whose path is set per command (:336, :392) and whose shared lock is taken per command (:407) and released in `afterCommand` (:727-737) → (c) | see E (lock) |

Other captured state: `SingleExtensionEvalFunction { services }` (`single_extension_eval_function.rs:101-105`), `RepositoryFetchFunction { services }` (`repository_fetch_function.rs:368-370`): only the `RepoServices` above. `setup.rs:67` `FetchSetup.reporter` goes into the `DownloadManager` (`setup.rs:431-436`) and the registry fetcher; not handed to functions otherwise. Statics read inside functions: only `Precomputed` key constants (fine) and the caches in E.

### C. File system reads that bypass FILE_STATE / DIRECTORY_LISTING_STATE (G2)

| Site | What | Java | Verdict |
|---|---|---|---|
| `crates/blaze_bzlmod/src/skyframe/module_file_function.rs:226-229` (+ `read_module_file` :1316-1326) | root `MODULE.bazel`: `exists()` + `std::fs::read`, no FILE dep (PORT note :226-227) | `BZ:ModuleFileFunction.java:296-301`: `env.getValue(FileValue.key(moduleFilePath))`, then the same `exists()`/`readModuleFile` | **G2 violation**: add the FileValue dep (blaze_bzlmod cannot depend on blaze_sky_loading per `precomputed.rs:3-8`; needs a seam like `PackageLookup`, or a crate move) |
| `module_file_function.rs:462-476` | include files: `is_file()` + `std::fs::read`, no FILE deps (PORT :462-463) | `BZ:ModuleFileFunction.java:494-518`: batch `FileValue` keys, `fileValue.isFile()`, then `FileSystemUtils.readContent` | **G2 violation** (same fix) |
| `module_file_function.rs:1021-1030` | non-registry-override module file: `read_module_file`, FILE dep skipped (PORT :1021) | `BZ:ModuleFileFunction.java:766`: `FileValue.key(moduleFilePath)` then read | **G2 violation** (same fix); the repo root comes from REPOSITORY_DIRECTORY, so a refetch already re-runs it, but an edit to a `local_path_override` module's `MODULE.bazel` would not |
| `module_file_function.rs:1216-1220` | patch files: `is_file()` per patch, no FILE dep; the patch bytes are read later by `apply_to_single_file` (:1285) | `BZ:ModuleFileFunction.java:916-931`: `FileValue` keys for each patch, `isFile()` check; `PatchUtil.applyToSingleFile` reads the real patch file too | **G2 violation** (missing deps; reading the patch after the dep is what Java does) |
| `crates/blaze_bzlmod/src/skyframe/lock_file.rs:48` (PORT :42-43) | lockfile `std::fs::read` | `BZ:BazelLockFileFunction.java:82` `env.getValue(FileValue.key(lockfilePath))` then `readContent` (:123) | **G2 violation** (same fix) |
| `module_file_function.rs:675, 686, 706` (`BuiltinPackageLookup::find_package`) | `.bazelignore`, `REPO.bazel`, `BUILD` stat/read | Java `PackageLookupFunction` via FileValue | stand-in not used in production (`setup.rs:555` installs `PackageLookupFunctionLookup` → PACKAGE_LOOKUP); dev bin/tests only. Remove or keep test-only. |
| `crates/blaze_repo_rules/src/repository/directory_tree_digest.rs:142-149` | `get_digest()` fallback on the real path | `J:.../skyframe/DirectoryTreeDigestFunction.java:80-84` same fallback after FileValue | allowed (Java does the same) |
| `crates/blaze_repo_rules/src/repository/repo_recorded_input.rs:645-651` | digest fallback | `J:com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:479-483` same | allowed (Java does the same) |
| `repository/repository_fetch_function.rs:627, 1042, 1214`; `repository/digest_writer.rs:121, 145, 149` (marker files); `repository/repository_utils.rs:31-35, 117, 167` (`isValidRepoRoot`, traverse, readlink); `starlark/base_external_context.rs`, `starlark/repository_context.rs`, `starlark/path.rs` (rctx/mctx file ops) | repository fetching and module extension contexts | REPOSITORY_DIRECTORY NonHermetic `S:133-134`; SINGLE_EXTENSION_EVAL NonHermetic `S:167-168`; Java reads the same paths (`RepositoryFetchFunction`, `DigestWriter`, `RepositoryUtils`, `StarlarkBaseExternalContext`) and re-checks via marker files / recorded inputs | allowed (Java does the same) |
| `crates/blaze_repo_rules/src/setup.rs:532` `process_wrapper.exists()` | at setup, not inside a function | `ProcessWrapper.fromCommandEnvironment` | not a G2 site |
| registry downloads (`registry_fetcher.rs`, `M:registry.rs` `FileRegistryFetcher` for `file:` URLs) | MODULE_FILE / REGISTRY / REPO_SPEC / YANKED_VERSIONS | all NonHermetic (`S:149, 175-178`); Java `DownloadManager.downloadAndReadOneUrlForBzlmod` reads `file:` URLs directly too | allowed (Java does the same) |

### D. Mutable or lazily filled state in values (G4)

| Site | Reachable from | Kind |
|---|---|---|
| `M:index_registry.rs:61` `bazel_registry_json: OnceLock<(Result<...>, StoredEventHandler)>` | REGISTRY value (`Arc<dyn Registry>`), and `InterimModule.registry` inside MODULE_FILE / BAZEL_MODULE_RESOLUTION values | memo, filled after the value is returned (first `get_module_file`/`get_repo_spec` call). Java same: `volatile Optional<BazelRegistryJson> bazelRegistryJson` + `bazelRegistryJsonEvents` (`BZ:IndexRegistry.java:103-104`). Fine, but it is why Java refreshes registries via `LAST_INVALIDATION` (B) and why row 1-2 identity matters (a pruned value keeps the old memo). |
| `crates/blaze_repo_rules/src/starlark/repository_module.rs:150, 152` `rule_name`, `repo_rule: OnceLock` | the `repository_rule` Starlark callable inside BZL_LOAD values, not this group's values (`RepoDefinition.repo_rule` holds the `Arc<RepoRule>` itself) | set on export / memo on first use (Java `StarlarkRepoRule` builder/`repoRule` field): fine; not owned by this group |
| `crates/blaze_bzlmod/src/module_file/mod.rs:132` `events: Mutex<Vec<Event>>` | module-file thread context during evaluation, not a value | fine |
| `crates/blaze_bzlmod/src/skyframe/resolution_function.rs:43-44` `RefCell`/`Cell` | local adapter during `compute` | fine |
| `crates/blaze_repo_rules/src/starlark/execution_result.rs:143, 272-278`, `base_external_context.rs:731`, `runnable_extension.rs:656` `Mutex` | rctx/mctx objects during evaluation, not values | fine |

No `Atomic*`, `RwLock` or `RefCell` in any value type of rows 1-19.

### E. Events, caches, side effects (G5, G6, G7)

| Kind | Site | Java | Note |
|---|---|---|---|
| G5 | `DownloadManager` built with the command reporter: `setup.rs:431-436`, `registry_fetcher.rs:46, 68` | same: `new DownloadManager(..., env.getReporter())` per command (`BazelRepositoryModule.java:276-281`) | download progress bypasses `env.listener()` in both; not replayed on a cache hit in Java either. Keep as Java. |
| G5 | `crates/blaze_repo_rules/src/starlark/thread.rs:44-53` `thread_local! LISTENER_ENVS` (print/warnings go to the SkyFunction env's listener) | Java print handler closes over `env.getListener()` | fine (RAII-scoped, events do reach the listener) |
| G5 | no `println!`/`eprintln!` in function code of either crate (grep; only `modcommand/` output formatters and `src/bin`) | | none |
| G6 | `crates/blaze_bzlmod/src/module_file/module_file_globals.rs:59` `MODULE_BAZEL_ENVS: LazyLock<Mutex<Vec<(StarlarkSemantics, Globals)>>>` | Java builds the module-file environment once per `ModuleFileFunction` | keyed by full semantics; fine (3.1 "keyed cache") |
| G6 | `crates/blaze_packages/src/bzl_context.rs:65` `BY_FILENAME` | none | **Surveyor correction:** not read in production. `repository_module.rs:531, 603` call the local trait method `of_filename_label` (`:644-652`), which uses `BazelModuleContext::of_innermost_bzl` (thread-based, as Java), not the table; `of_filename`/`all_package_contexts` have no non-test caller (grep `of_filename\|all_package_contexts` over `crates`). Memory only (plan 3.1 is stale here). No I2 work. |
| G7 | `module_file_function.rs:229-232` `create_module_file` writes `MODULE.bazel` into the workspace when missing | same `BZ:ModuleFileFunction.java:303-304` | allowed (Java does the same) |
| G7 | `module_file_function.rs:1238-1252` + `:1256-1313`: patched copy written to `<output_base>/tmp/module-file-patch-<uuid>/module/MODULE.bazel`, removed after (`remove_dir_all` result ignored :1250) | `BZ:ModuleFileFunction.java:936-955` uses an `InMemoryFileSystem` | repeatable (fresh uuid per run), but writes to the real FS, can leak dirs on error/abort, and error texts are rewritten (`:1268-1269`). Needs an in-memory `blaze_vfs` FS for `PatchUtil` to match Java. |
| G7 | lockfile write: `lock_file.rs:63-209` `after_command`, `update_lockfile` :212 (I/O errors dropped, PORT :237), called from `crates/blaze_sky_top/src/skyframe_executor.rs:597` | `BZ:BazelLockFileModule.java:63` `afterCommand`, `updateLockfile` :336 (logs SEVERE) | outside functions, same as Java; Java's next command sees the write through the lockfile `FileValue` diff, which Rust lacks until the C fix at `lock_file.rs:48` |
| G7 | repo marker files `repository/digest_writer.rs:121` (write), clear `repository_fetch_function.rs:525, 1139`; repo dir deletes/creates in `repository_fetch_function.rs`, `starlark/repository_context.rs`, `starlark/base_external_context.rs` | `DigestWriter`, `RepositoryFetchFunction` | allowed (Java does the same) |
| G7 | process spawns: `base_external_context.rs:2160-2170` (rctx/mctx `execute` via process-wrapper) | `StarlarkBaseExternalContext.execute` | allowed (non-hermetic functions) |
| G7 | repo contents cache shared lock: `setup.rs:420-428` acquires once, never released (PORT :423-425); `release_shared_lock` exists unused at `crates/blaze_repo_fetch/src/cache/local_repo_contents_cache.rs:187` | acquire in `beforeCommand` (`BazelRepositoryModule.java:407`), release in `afterCommand` (:727-737) | must move to before/after command for a server (3.2 item 5) |
| G7 | network: registry and repository downloads | NonHermetic functions in Java | allowed |

### Greps run

- `grep -rnE "impl\s*(<[^>]*>)?\s*(blaze_skyframe::)?Key\s+for" crates/blaze_bzlmod crates/blaze_repo_rules` (19 hits, none in tests)
- Java: `grep -nE "class|record|@AutoValue|equals|hashCode|NotComparable|implements|extends"` over each value class in `BZ:`; `grep -n "boolean equals"` over `BZ:*.java` (only `Version.java:199`), `Dict.java`, `Event.java`, `Path.java`, `Location.java`, `RepositoryMapping.java`, `PrecomputedValue.java`
- `grep -n -A1 -E "PRECOMPUTED|CLIENT_ENVIRONMENT|REPOSITORY_ENVIRONMENT|MODULE_FILE|BAZEL_|SINGLE_EXTENSION|REGISTRY|REPO_SPEC|YANKED|MODULE_EXTENSION_REPO|DIRECTORY_TREE|REPO_DEF|REPOSITORY_DIRECTORY" S:`; `grep -rln REPOSITORY_ENVIRONMENT_VARIABLE J:com/google/devtools/build/lib`
- `grep -n "LAST_INVALIDATION|lastRegistryInvalidation|INVALIDATION_INTERVAL|MODULE_MIRRORS|repoEnv|nonstrictRepoEnv|handleClientEnvironmentChanges|handleRepositoryEnvironmentChanges"` in `BazelRepositoryModule.java`, `SkyframeExecutor.java`, `RegistryFunction.java`, `bazel/repository/*.java`
- Rust derives/manual eq: `grep -rnE -B4 "(struct|enum|trait) <T>"` in `M:`; `grep -rn "impl.*PartialEq for|impl Eq for" M:`
- `grep -nE "config\.[a-z_]+"` and `package_lookup|repository_directory_lookup|registry_factory|client_env|LAST_INVALIDATION` in `crates/blaze_bzlmod/src/skyframe/*.rs`; `grep -rnE "\.(repo_env|nonstrict_repo_env|download_manager|timeout_scaling|process_wrapper|repo_contents_cache|directories)\b"` in blaze_repo_rules
- `grep -rn "client::<|\.client()"` (none)
- FS: `grep -rnE "std::fs|fs::|read_to_string|\.exists\(|\.is_file\(|\.is_dir\(|metadata\(|read_dir|read_link|readlink|File::open|File::create|\.stat\(|get_digest|PORT:"` over blaze_bzlmod; same plus `is_directory|get_directory_entries|read_symbolic_link|FileSystemUtils|read_content|write_content|Command::new|spawn` over blaze_repo_rules; `remove_dir_all|create_dir_all|delete_tree|create_directory_and_parents|write_content|create_symbolic_link|rename_to`
- Mutability: `grep -rnE "OnceLock|OnceCell|Mutex|RwLock|Atomic[A-Z]|RefCell|\bCell<|LazyLock|thread_local"` over blaze_bzlmod, blaze_bzlmod_model, blaze_repo_rules
- Output: `grep -rnE "eprintln!|println!|eprint!|stderr\(\)|io::stdout|reporter|event_handler\b"`; statics: `grep -rn "static "`
- Callers: `register_bzlmod_functions`, `from_repository_options`, `impl .*PackageLookup for|RepositoryDirectoryLookup for`, `after_command(`, `acquire_shared_lock|release_shared_lock`, `ClientEnvironmentKey::key|RepoEnvironmentKey::key|get_environment_view(`, `register_lockfile_only_extension_functions` (dev bin only)

### Not checked

- Field-by-field equality of `ModuleExtensionUsage` (`M:module_extension.rs:260`, derived) vs AutoValue incl. `Proxy`, `Tag`, `Location`, `RepoOverride` records; `ModuleOverride`/`SingleVersionOverride` fields; `Version::eq` body vs `Version.java:199`; `ModuleKey`/`ModuleExtensionId` eq; `StarlarkSemantics` `PartialEq` (needed for EQ-WRITE of row 12); `StarlarkFloat.equals` NaN semantics vs `FactValue::Float`.
- Whether any production code calls `Precomputed::set_on` per command.
- REPOSITORY_DIRECTORY / REPOSITORY_MAPPING keys (registered by blaze_sky_loading over `BzlmodExternalRepos`, not in this group's key list) and the internals of `RepositoryFetchFunction` beyond the FS-site grep.
- `SingleExtensionEvalFunction` and `RunnableExtension` internals beyond captured state and the grep lists.
- The 15 static items 3.1 counted in blaze_repo_rules were not re-opened one by one; grep of `static ` found only `Precomputed` constants, `MethodsStatic`, `LISTENER_ENVS`, and a test-only `FS` (`starlark/path.rs:242`, inside `#[cfg(test)]`).

### F. Group 2 worker: files, order, hand-back

Files:
- `crates/blaze_bzlmod/src/skyframe/{keys.rs, precomputed.rs, config.rs, mod.rs, module_file_function.rs, lock_file.rs, registry_functions.rs}`
- `crates/blaze_bzlmod/src/inspector.rs`
- `crates/blaze_bzlmod_model/src/module.rs` (`InterimModule::eq`: compare `registry` by `Arc::ptr_eq`)
- `crates/blaze_bzlmod_model/src/external_deps_exception.rs` (only if errors become `Arc` for identity)
- `crates/blaze_repo_rules/src/{services.rs, setup.rs, registration.rs}`
- `crates/blaze_repo_rules/src/repository/{repo_environment_function.rs, repo_definition.rs, directory_tree_digest.rs}`
- `crates/blaze_repo_rules/src/bzlmod/{single_extension_usages.rs, repo_mapping_entries.rs, bazel_fetch_all_function.rs}`

Order:
1. Adopt X1. Delete `crates/blaze_bzlmod/src/skyframe/precomputed.rs`'s own `PrecomputedKey` and use the
   shared one. `ClientEnvironmentKey` and `EnvironmentVariableValue` move with it (X2).
2. G2 seam: a `FileValue` lookup trait in `blaze_bzlmod`, implemented over `FileKey` in
   `crates/blaze_repo_rules/src/setup.rs` next to `PackageLookupFunctionLookup` (`:585`, `:623`). Use
   it at the 5 sites in C: root `MODULE.bazel`, includes, non-registry override module file, patch
   files, lockfile. Without this, editing `MODULE.bazel` does not re-run resolution.
3. `HERMETICITY` on the 19 keys (8 NonHermetic).
4. `InterimModule.registry` by pointer. After that, `ModuleFileKey` and `BazelModuleResolutionKey`
   become EQ-OK.
5. Opt in the 12 EQ-OK keys. Write equality for the inspector and mod-tidy values (`errors` equal only
   when both are empty).
6. G1:
   - `client_env`, `repo_env`, `nonstrict_repo_env`, `registry_fetcher`/download manager,
     timeout scaling and process wrapper become per-command cells (setters, as in
     `BazelRepositoryModule.beforeCommand`);
   - per-variable injection of the env values belongs to I3, but the closures must read cells and not
     snapshots;
   - `last_registry_invalidation`: port the module field and the 1-hour rule
     (`J:com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:156, 668-675, 746-749`) in
     place of `SystemTime::now()`.
7. G7:
   - the repo contents cache lock is released in `afterCommand` (`setup.rs:420-428`);
   - the patched module file is written to an in-memory FS (`module_file_function.rs:1238-1313`). Its
     priority is low; record it as a PORT if it is deferred.

Hand back: one line per key, as for group 1. For `last_registry_invalidation`, a test that a second
command in REFRESH mode within an hour does not re-run REGISTRY. A test per G2 site that editing the
file dirties the node (it needs I1: until then, a test that the dependency edge is recorded).

## Group 3: blaze_build_config (7 keys)

Helper draft: **Group 3: `blaze_build_config` (7 keys)**

Tree: HEAD `4f92912` plus uncommitted work of others (line numbers as of 2026-11-19). Read-only survey; nothing built.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`. All `SkyFunctions` lines are `J:com/google/devtools/build/lib/skyframe/SkyFunctions.java`.

### A. Keys

| # | Rust key | FUNCTION | Rust value type | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `crates/blaze_build_config/src/platform.rs:48` `PlatformValueKey` | `PLATFORM` | `PlatformValue` `platform.rs:55` | derived `platform.rs:54`. Fields: `label`, `constraints` (projection of `PlatformInfo`), `has_parsed_flags`, `parsed_flags: Option<Arc<ParsedFlagsValue>>` (content eq via `ParsedFlagsValue::eq`), `platform_info: Option<OpaquePlatformInfo>`, compared by `Arc::ptr_eq` (manual `platform.rs:76`) | `J:com/google/devtools/build/lib/analysis/platform/PlatformValue.java:61` record `Key` | `J:.../analysis/platform/PlatformValue.java:35` **record** `(PlatformInfo platformInfo, Optional<ParsedFlagsValue> parsedFlags)`. The record equals uses `PlatformInfo.equals` (`J:.../analysis/platform/PlatformInfo.java:182`, value equality over label, constraints, remote/exec properties, flags, requiredSettings, toolchain types, missing-toolchain message) and `ParsedFlagsValue.equals` (`:226`) | Hermetic, `SkyFunctions.java:129` | **MIXED** (prelim.). (1) Real `PlatformFunction` (`crates/blaze_sky_analysis/src/platform/platform_function.rs:147`) always sets `platform_info: Some(..)` from a fresh `Arc<PlatformInfo>`, so a recompute never compares equal to the old value: weaker than Java (no pruning where Java prunes). Rust `PlatformInfo` already has a Java-faithful `PartialEq` (`crates/blaze_analysis/src/platform/platform_info.rs:220`); the fix is an equality function carried in `OpaquePlatformInfo` (downcast both and compare), because `blaze_build_config` cannot name the type. (2) The oracle path (`OraclePlatforms`, `platform.rs:175-181`, `platform_info: None`) compares only label + CPU-style constraints + a flags boolean, so it ignores the other `PlatformInfo` fields and flag contents: stronger than Java. Today that path cannot carry flags (`unported!` at `platform.rs:212`), so (2) only matters for exec properties etc. It is a fixed per-server file, so the risk is low. Write `values_equal` by hand. Do not opt in with the derived `==`. |
| 2 | `crates/blaze_build_config/src/build_options_scope_value.rs:82` `BuildOptionsScopeKey` | `BUILD_OPTIONS_SCOPE` | `BuildOptionsScopeValue` `build_options_scope_value.rs:91` | none (`Clone, Debug`) | `J:.../skyframe/BuildOptionsScopeValue.java:38` (`equals` :75) | `J:.../skyframe/BuildOptionsScopeValue.java:29`: no `equals`, so identity (fields package-private and mutable) | Hermetic, `SkyFunctions.java:185-186` | **IDENTITY** (prelim.). Leave the default. |
| 3 | `crates/blaze_build_config/src/parsed_flags.rs:95` `ParsedFlagsKey` | `PARSED_FLAGS` | `ParsedFlagsValue` `parsed_flags.rs:211` | manual `parsed_flags.rs:345` (`flags == flags`), calling `NativeAndStarlarkFlags::eq` manual `parsed_flags.rs:161` | `J:.../skyframe/config/ParsedFlagsValue.java:47` record `Key` | `J:.../skyframe/config/ParsedFlagsValue.java:42`, `equals` at :226 compares `flags` only (`NativeAndStarlarkFlags`, `@AutoValue` `J:.../skyframe/config/NativeAndStarlarkFlags.java:32`) | Hermetic, `SkyFunctions.java:114` | **EQ-OK** (prelim.). Rust ignores `parsing_result`, as Java does. Minor: `options_classes` is compared in order by pointer (`parsed_flags.rs:168-173`) where Java's `ImmutableSet.equals` ignores order. Only reorderings differ (the list comes from the server-constant fragment registry), and Rust is the stricter side, so no wrong pruning. Starlark values use `java_equals`. `RepositoryMapping` has a manual map-equality `crates/blaze_cmdline/src/repository_mapping.rs:171` (Java `RepositoryMapping.equals` `J:.../cmdline/RepositoryMapping.java:65`). |
| 4 | `crates/blaze_build_config/src/skyframe.rs:34` `BuildConfigurationKey` | `BUILD_CONFIGURATION` | `BuildConfigurationValue` `crates/blaze_build_config/src/build_configuration_value.rs:155` | manual `build_configuration_value.rs:652`: `build_options` (checksum), `workspace_name`, `sibling_repository_layout`, `mnemonic` | `J:.../skyframe/config/BuildConfigurationKey.java:29` (`equals` :63, options) | `J:.../analysis/config/BuildConfigurationValue.java:89`, `equals` at :341, the same four fields ("non-dependent and non-server-global") | Hermetic, `SkyFunctions.java:110-111` | **EQ-OK** (prelim.). A field-for-field match. `BuildOptions::eq` (`crates/blaze_build_config/src/build_options.rs:801`) is ptr-or-checksum, as in Java `BuildOptions.equals` (`J:.../analysis/config/BuildOptions.java:273`). Note: registered with an untyped `.function(..)` (`skyframe.rs:1293`), not `typed_function`, so the I1 opt-in must also reach untyped registrations. |
| 5 | `crates/blaze_build_config/src/skyframe.rs:39` `BuildConfigurationKeyValueKey` | `BUILD_CONFIGURATION_KEY` | `BuildConfigurationKey` `crates/blaze_build_config/src/build_configuration_key.rs:13` (stands in for `BuildConfigurationKeyValue`) | manual `build_configuration_key.rs:35` (`*options == *options`, checksum) | `J:.../skyframe/config/BuildConfigurationKeyValue.java:34` (`equals` :63) | `J:.../skyframe/config/BuildConfigurationKeyValue.java:29`, `equals` at :105 compares `buildConfigurationKey`, which compares `options` (`BuildConfigurationKey.java:63-70`) | Hermetic, `SkyFunctions.java:112-113` | **EQ-OK** (prelim.). |
| 6 | `crates/blaze_build_config/src/skyframe.rs:65` `BaselineOptionsKey` | `BASELINE_OPTIONS` | `Arc<BuildOptions>` (`build_options.rs:424`) | `Arc<T>: PartialEq` compares contents, then `BuildOptions::eq` manual `build_options.rs:801` (ptr or checksum) | `J:.../analysis/config/transitions/BaselineOptionsValue.java:68` record `Key` | `J:.../analysis/config/transitions/BaselineOptionsValue.java:50` **record** `(BuildOptions toOptions)`, so `BuildOptions.equals` (`J:.../analysis/config/BuildOptions.java:273`, checksum) | **NonHermetic**, `SkyFunctions.java:115-116` | **EQ-OK** (prelim.). Hermeticity must be `NonHermetic`: it reads the BASELINE precomputeds. |
| 7 | `crates/blaze_build_config/src/skyframe.rs:85` `PlatformMappingSkyKey` | `PLATFORM_MAPPING` | `PlatformMappingValue` `skyframe.rs:96` | derived `skyframe.rs:95`: `platforms_to_flags: IndexMap` (order-insensitive map eq), `flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)>` (order-**sensitive**) | `J:.../skyframe/config/PlatformMappingKey.java:29` (`equals` :80) | `J:.../skyframe/config/PlatformMappingValue.java:50`, `equals` at :162: `flagsToPlatforms` and `platformsToFlags` (`ImmutableMap`, order-insensitive), plus `optionsClasses` | Hermetic, `SkyFunctions.java:130-131` | **EQ-OK** (prelim.), with one caveat. Rust drops `optionsClasses`. That is a constructor argument of `PlatformMappingFunction` (`J:.../skyframe/config/PlatformMappingFunction.java:64`), a server constant, so it is equal for any two computations. The `Vec` makes a reordered mapping file compare unequal where Java prunes: Rust is the stricter side, and it is never wrong. To match Java exactly, compare `flags_to_platforms` as a map (keys are unique). Java's `mappingCache` (`LoadingCache`, :54) is not ported (`skyframe.rs:94`) and does not take part in equality. |

Verdict counts: EQ-OK 5, MIXED 1, IDENTITY 1.

Name check: all 7 Rust FUNCTION strings equal the Java `SkyFunctions` names.

### B. Inputs that are not dependencies (G1)

Registration: `register_configuration_functions` `crates/blaze_build_config/src/skyframe.rs:1291`, called with `.client(ConfigurationEnvironment)` at `crates/blaze_sky_top/src/skyframe_executor.rs:407`. The struct is built at `skyframe_executor.rs:393-406`, once per command (one executor per command today). It is read through `client(env)` (`skyframe.rs:231`).

| Field (`crates/blaze_build_config/src/skyframe.rs`) | Read at | Class | Java |
|---|---|---|---|
| `directories: ConfigDirectories` :216 | `:1185` (`BuildConfigurationValue::create`) | (a) server constant | `BuildConfigurationFunction.directories` constructor field `J:.../skyframe/config/BuildConfigurationFunction.java:45` |
| `rule_class_provider` :217 | `:1186` | (a) | `BuildConfigurationFunction.ruleClassProvider` `:46` |
| `starlark_semantics` :219 | `:1168` (BUILD_CONFIGURATION) | (b) `PrecomputedValue.STARLARK_SEMANTICS` `J:.../skyframe/PrecomputedValue.java:85`, read at `BuildConfigurationFunction.java:59` | must become a precomputed dep: a `--experimental_sibling_repository_layout` change must invalidate BUILD_CONFIGURATION |
| `package_path_roots` :221 | `:263` (PLATFORM_MAPPING) | (b) `PrecomputedValue.PATH_PACKAGE_LOCATOR` `PrecomputedValue.java:93`, read at `J:.../skyframe/config/PlatformMappingFunction.java:84` | precomputed dep (the loading crate already injects `PATH_PACKAGE_LOCATOR`; read that instead) |
| `baseline_configuration` :223 | `:912` (BUILD_CONFIGURATION_KEY scopes path, Java `BuildConfigurationKeyProducer.java:287` `tasks.lookUp(BASELINE_CONFIGURATION.getKey())`), `:1063` (BASELINE_OPTIONS) | (b) `PrecomputedValue.BASELINE_CONFIGURATION` `PrecomputedValue.java:100`, set in `SkyframeExecutor.setBaselineConfiguration` `J:.../skyframe/SkyframeExecutor.java:1536-1541` (`:1539`) | precomputed dep |
| `baseline_exec_configuration` :225 | `:1060` (BASELINE_OPTIONS) | (b) `PrecomputedValue.BASELINE_EXEC_CONFIGURATION` `PrecomputedValue.java:104`, set at `SkyframeExecutor.java:1540` | precomputed dep. Note: it is computed by `adjust_for_exec` with the command's reporter outside any function (`skyframe_executor.rs:387-391`), as in Java `adjustForExec(buildOptions, eventHandler)` |
| `platforms: SharedPlatformValueProvider` :226 | `:375`, `:1107`, `:1301` (PLATFORM function body) | (a) the function implementation (`PlatformFunction` holding `AnalysisServices`, `skyframe_executor.rs:383-385`). Its captured `AnalysisServices` fields are the G1 row of `blaze_analysis` (other group) | Java `PlatformFunction` registered in `SkyframeExecutor.skyFunctions` |
| `main_repository_mapping` :228 | `:259` | (a) an adapter that requests `RepositoryMappingKey(MAIN)` through `env` (`MainRepositoryMappingFromSkyframe`), so it is a real dep | `PlatformMappingFunction.java:77` |

Other captured state of this group's functions, registered elsewhere:
- `ParsedFlagsFunction` captures `options_classes` (`crates/blaze_sky_analysis/src/parsed_flags_function.rs:36-37`, registered at `skyframe_executor.rs:410-417`): (a) server constant (fragment registry). Owner: `blaze_sky_analysis`.
- `BuildOptionsScopeFunction` (`crates/blaze_sky_analysis/src/build_options_scope_function.rs:29`): no captured state. Owner: `blaze_sky_analysis`.

Not a G1 problem, but per-command state of the same subsystem: `ConfigurationState.baseline` (`skyframe_executor.rs:138-141`) and the module comment `skyframe_executor.rs:5-10` say the baseline is fixed at executor construction. With a server-lifetime executor, `setBaselineConfiguration` must re-inject per command (Java `SkyframeExecutor.java:1536`).

### C. File-system reads bypassing FILE_STATE (G2)

| Site | What | Java | Note |
|---|---|---|---|
| `crates/blaze_build_config/src/skyframe.rs:270` `path.stat_if_found(Follow)` (+ `:281` `is_directory()`) | stat of `<root>/<--platform_mappings>` per package path root | `J:.../skyframe/config/PlatformMappingFunction.java:92` `env.getValue(FileValue.key(rootedMappingPath))`, then `exists()`/`isDirectory()` | **Must fix** (marked `PORT: skipped, incremental` at `:265`). Without the FILE edge, creating, editing or deleting the mapping file never invalidates PLATFORM_MAPPING. The FILE key lives in `blaze_sky_loading`, which `blaze_build_config` cannot name. It needs a seam like `MainRepositoryMappingLookup` (`:228`). |
| `crates/blaze_build_config/src/skyframe.rs:291` `path.get_input_stream()` + `read_to_end` | reads the mapping file's contents | Java reads directly too: `FileSystemUtils.readLines` after the FileValue dep (`PlatformMappingFunction.java` around :110) | **Allowed (Java does the same)** once the FILE dep at `:270` exists, since the FILE_STATE of the file covers content changes. |
| `crates/blaze_build_config/src/platform.rs:195` `std::fs::read_to_string` (`OraclePlatforms::load`) | oracle platform facts | none (Blaze test seam) | Not in a SkyFunction: only called from `src/bin/blaze-config-dev.rs:232` (excluded). |
| `crates/blaze_build_config/src/bin/blaze-config-dev.rs:384` | dev binary | n/a | excluded (`src/bin`). |

No other `std::fs`/`blaze_vfs` reads in `crates/blaze_build_config/src` (see greps).

### D. Mutable / lazily filled state in values (G4)

| Site | Reached from | Kind | Verdict |
|---|---|---|---|
| `crates/blaze_build_config/src/build_options.rs:435` `checksum: OnceLock<String>` | `BuildConfigurationValue.build_options`, `BaselineOptions` value, `BuildConfigurationKey`, `BuildOptionsScopeValue` | lazy memo (Java `BuildOptions.checksum` double-checked lazy field, `J:.../analysis/config/BuildOptions.java:180-198`) | memo, fine. Caveat: `BuildOptions::get_mut` (`build_options.rs:563`) does not reset an already computed checksum (documented, Java-identical). It needs `&mut`, so it cannot reach a value behind `Arc` in the graph unless someone `Arc::get_mut`/`make_mut`s it. No such site seen, but not exhaustively checked. |
| `crates/blaze_options/src/priority.rs:44` `already_expanded: AtomicBool` (inside `OptionPriority`, shared handle) | `ParsedFlagsValue.parsing_result: OptionsParser` (`parsed_flags.rs:213`), also in `PlatformMappingValue` and `PlatformValue.parsed_flags` | set by `get_child_priority` during parsing (Java `OptionPriority.alreadyExpanded`, same) | Fine in practice: written only while parsing, before the value is returned. Mutation after return was not seen (needs I2 confirmation that nothing re-parses with a stored parser). |
| `crates/blaze_options/src/definition.rs:117-118` `OnceLock` converter / default value | option definitions (statics) reachable from the stored `OptionsParser` | memo | fine |
| `crates/blaze_options/src/data.rs:28` `OPTIONS_DATA_CACHE` static `Mutex<HashMap>` | `OptionsParser` construction in `NativeAndStarlarkFlags::parse` | cache keyed by option class set | fine: class definitions are server constants (Java has a static `OptionsParser` data cache) |
| `PlatformValue.platform_info` (`platform.rs:67`) holds `Arc<PlatformInfo>`. Searched `crates/blaze_analysis/src/platform/*.rs` for interior mutability | | none found | fine |
| `BuildConfigurationValue.fragments: Vec<(FragmentClass, Arc<dyn Fragment>)>` (`build_configuration_value.rs:157`) | | no `Cell`/`Mutex`/`OnceLock`/`Atomic` in `crates/blaze_build_config/src/fragments/` | fine |
| Java-only memos not ported: `ParsedFlagsValue.mergeCache` (`J:.../skyframe/config/ParsedFlagsValue.java:120`), `PlatformMappingValue.mappingCache` (`PlatformMappingValue.java:54`, PORT `skyframe.rs:94`) | | absent in Rust | performance only |

### E. Events, caches, side effects (G5/G6/G7)

| Site | Kind | Java | Verdict |
|---|---|---|---|
| `crates/blaze_build_config/src/skyframe.rs:1191-1194` `env.listener().post(ConfigurationValueEvent)` | event through `env.listener()` | `BuildConfigurationFunction` posts the same event, and `storeForReplay()` is not overridden (`skyframe.rs:1149`), so it is not replayed. Same in Java. | fine (G5) |
| `crates/blaze_build_config/src/transitions.rs:18` `static EMPTY_OPTIONS: LazyLock<BuildOptions>` | static constant | `CommonOptions.EMPTY_OPTIONS` `J:.../analysis/config/CommonOptions.java:32` | fine (constant) |
| `crates/blaze_build_config/src/transitions.rs:47` `BuildOptionsCache` of `TestTrimmingLogic.trim` not ported | cache absent | `J:.../analysis/test/TestTrimmingLogic.java:45` `static BuildOptionsCache<Boolean> CACHE` | performance only. With no cache there is no G6 risk. Optional port. |
| `crates/blaze_build_config/src/fragment.rs:321-322` `FragmentFactory` cache not ported | cache absent | `FragmentFactory` weak cache | performance only; `BuildConfigurationValue::eq` does not compare fragments |
| `crates/blaze_build_config/src/fragments/test_configuration.rs:4` `SHOULD_INVALIDATE_FOR_OPTION_DIFF` skipped | missing invalidation predicate | `TestConfiguration.SHOULD_INVALIDATE_FOR_OPTION_DIFF` | I5 (option-diff invalidation), not I2 |
| `crates/blaze_build_config/src/skyframe.rs:1488-1505` `ErrorSensing` with an `AtomicBool` | per-call handler in `create_configuration` (executor method, not a SkyFunction) | `ErrorSensingEventHandler` | fine |
| `eprintln!`/`println!`/stderr | none in `crates/blaze_build_config/src` outside `src/bin` | | fine |
| G7 (fs writes, network, processes) in configuration functions | none | | fine |

### Greps run
- `grep -n 'impl.*Key for\|impl Key\|const FUNCTION\|type Value' crates/blaze_build_config/src/{platform,build_options_scope_value,parsed_flags,skyframe}.rs`, and `grep -rn 'impl.*Key for' crates/blaze_build_config/src` (no other production keys)
- `grep -n 'equals\|hashCode\|record\|AutoValue\|NotComparable' J:` PlatformValue, PlatformInfo, BaselineOptionsValue, BuildOptionsScopeValue, BuildConfigurationKey(Value), ParsedFlagsValue, NativeAndStarlarkFlags, PlatformMappingKey/Value, BuildConfigurationValue, BuildOptions, RepositoryMapping
- `grep -n -E 'PLATFORM|BUILD_OPTIONS_SCOPE|PARSED_FLAGS|BUILD_CONFIGURATION|BASELINE_OPTIONS' J:.../skyframe/SkyFunctions.java`
- `grep -rn 'OnceLock\|OnceCell\|Mutex\|RwLock\|Atomic\|RefCell\|Cell<\|LazyLock\|static ' crates/blaze_build_config/src`; the same over `crates/blaze_options/src` and `crates/blaze_analysis/src/platform`
- `grep -rn 'std::fs\|read_to_string\|\.exists()\|stat_if_found\|\.stat(\|read_dir\|readlink\|is_file()\|is_directory()\|is_dir()\|metadata\|get_input_stream\|File::open' crates/blaze_build_config/src`
- `grep -rn 'eprintln!\|println!\|stderr()\|stdout()' crates/blaze_build_config/src`
- `grep -rn 'PORT:[^\n]*incremental\|incremental' crates/blaze_build_config/src`
- `grep -n 'client(env)' crates/blaze_build_config/src/skyframe.rs`; `grep -rn 'ConfigurationEnvironment {' crates`
- `grep -n 'PrecomputedValue\|getValue(' J:` BuildConfigurationFunction, BaselineOptionsFunction, BuildConfigurationKeyFunction, BuildConfigurationKeyProducer, PlatformMappingFunction
- `grep -rn 'values_equal\|HERMETICITY' crates/blaze_skyframe/src`: nothing yet (I1 not in the tree)

### Not checked
- Whether any code calls `Arc::get_mut`/`make_mut` on a `BuildOptions` held by a graph value after return.
- The full body of `BuildConfigurationKeyProducer`/`apply_scopes` (`skyframe.rs:880-1045`) for further inputs beyond the `baseline_configuration` read.
- `starlark_transition.rs` / `function_transition_util.rs` (exec transition loading, which runs outside SkyFunctions here), apart from the `adjust_for_exec` call site.
- `AnalysisServices` fields captured by `PlatformFunction` (owner `blaze_analysis`/`blaze_sky_analysis`, other group).
- How I1 will attach `values_equal` to keys registered with untyped `.function(..)` (BUILD_CONFIGURATION, `skyframe.rs:1293`).

### F. Group 3 worker: files, order, hand-back

Files:
- `crates/blaze_build_config/src/{platform.rs, build_options_scope_value.rs, parsed_flags.rs, skyframe.rs}`
- `crates/blaze_sky_analysis/src/platform/platform_function.rs`: it creates `OpaquePlatformInfo`, and
  this group needs the equality function stored there. Coordinate with group 4, which owns the crate.
- The `ConfigurationEnvironment` construction at `crates/blaze_sky_top/src/skyframe_executor.rs:393-407`
  is requested from group 6.

Order:
1. `HERMETICITY` on the 7 keys. `BASELINE_OPTIONS` is NonHermetic.
2. Opt in the 5 EQ-OK keys.
3. `PlatformValue` (MIXED): add an equality function to `OpaquePlatformInfo` (downcast both sides and
   use `PlatformInfo::eq`, `crates/blaze_analysis/src/platform/platform_info.rs:220`), then write
   `values_equal` as the record equality. Do not use the derived `==` (pointer compare plus the
   oracle-path projection).
4. Leave `BuildOptionsScopeValue` at the default (IDENTITY).
5. G1, after X1: `starlark_semantics`, `package_path_roots` (read `PATH_PACKAGE_LOCATOR`),
   `baseline_configuration` and `baseline_exec_configuration` become precomputed dependencies, so that
   `ConfigurationEnvironment` keeps only the server constants (`directories`, `rule_class_provider`,
   `platforms`, `main_repository_mapping`).
6. G2: request the `FILE` node of the platform mappings file through a seam (`skyframe.rs:270`; Java
   `J:.../skyframe/config/PlatformMappingFunction.java:92`). The seam implementation lives in
   `blaze_sky_top` or `blaze_sky_analysis`, both of which can name `FileKey`.
7. Optional: compare `PlatformMappingValue.flags_to_platforms` as a map (exact Java).

Hand back: one line per key. Per `ConfigurationEnvironment` field: its class and what was done.
Tests:
- `PlatformValue` equality on a recompute, which must prune;
- a changed `--experimental_sibling_repository_layout` (semantics), which must dirty
  BUILD_CONFIGURATION;
- an edit of the platform mappings file, which must dirty PLATFORM_MAPPING.

## Group 4: blaze_analysis + blaze_sky_analysis (10 keys)

Helper draft: **I2 survey, group 4: `blaze_analysis` + `blaze_sky_analysis`**

Tree: HEAD `4f92912` plus uncommitted work (line numbers as of 2026-11-19). Read-only; no Cargo.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`. All verdicts are preliminary.

### A. Keys (10 production `impl Key`; 7 in `blaze_analysis`, 3 in `blaze_sky_analysis`)

| # | Rust key (`file:line`) | FUNCTION | Rust value type (def) | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `crates/blaze_analysis/src/configured_target_key.rs:199` `ConfiguredTargetKey` | `CONFIGURED_TARGET` (`configured_target_key.rs:16`) | `ConfiguredTargetValue` `crates/blaze_analysis/src/configured_target_value.rs:18` (one struct for Rule and NonRule) | none (`#[derive(Clone, Debug)]` at :17) | `J:com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:60` | `J:.../skyframe/RuleConfiguredTargetValue.java:31`, `NonRuleConfiguredTargetValue.java:35`, both through `AbstractConfiguredTargetValue.java:25` implementing `analysis/ConfiguredTargetValue.java:21`, which extends `analysis/ConfiguredObjectValue.java:24` `extends NotComparableSkyValue`. No `equals` anywhere in the chain | Hermetic, `J:.../skyframe/SkyFunctions.java:95-96` | `IDENTITY` (prelim.): NotComparable |
| 2 | `crates/blaze_analysis/src/analysis_services.rs:75` `WorkspaceStatusKey` | `BUILD_INFO` (string const `crates/blaze_actions/src/analysis/workspace_status_action.rs:27`, used at `analysis_services.rs:78-80`) | `WorkspaceStatusValue` `analysis_services.rs:87` | none (`#[derive(Debug)]` :86) | `J:.../skyframe/WorkspaceStatusValue.java:55` `BuildInfoKey` (singleton `BUILD_INFO_KEY` :39) | `J:.../skyframe/WorkspaceStatusValue.java:33` `extends BasicActionLookupValue` (`J:.../actions/BasicActionLookupValue.java:25`); neither has `equals`: identity | Hermetic, `SkyFunctions.java:128` | `IDENTITY` (prelim.): no equals. Java's node has no deps, so it is computed once per server and never dirtied (`WorkspaceStatusFunction.java:38-50`) |
| 3 | `crates/blaze_analysis/src/toolchains/toolchain_context_key.rs:103` `ToolchainContextKey` | `TOOLCHAIN_RESOLUTION` (`toolchain_context_key.rs:13`) | `UnloadedToolchainContextImpl` `crates/blaze_analysis/src/toolchains/unloaded_toolchain_context.rs:22` | derived (`:21`) | `J:.../skyframe/toolchains/ToolchainContextKey.java:31` (`@AutoValue`; `functionName()` :44-45 = `TOOLCHAIN_RESOLUTION`) | `J:.../skyframe/toolchains/UnloadedToolchainContextImpl.java:31-32` `@AutoValue`: equals over `key, executionPlatform, targetPlatform, toolchainTypes (ImmutableSet), toolchainTypeToResolved (ImmutableSetMultimap), requestedLabelToToolchainType (ImmutableMap), errorData` | Hermetic, `SkyFunctions.java:145-146` | `EQ-OK` (prelim.). Rust compares the same fields. `Arc<PlatformInfo>` compares by value through the manual `PlatformInfo::eq` (`crates/blaze_analysis/src/platform/platform_info.rs:220`), which matches `J:.../analysis/platform/PlatformInfo.java:182-194` field by field. Caveat: `toolchain_types` and `toolchain_type_to_resolved` are `Vec`s, and so is `NoMatchingPlatformData.toolchain_types` (`toolchains/no_matching_platform_data.rs:13`). Rust compares them in order; Java's set and multimap equality does not. So Rust can only say "unequal" where Java says equal. That costs a prune but never prunes wrongly. A manual `eq` would make it exact |
| 4 | `crates/blaze_analysis/src/config/starlark_transition.rs:101` `StarlarkBuildSettingsDetailsKey` | `STARLARK_BUILD_SETTINGS_DETAILS` (`starlark_transition.rs:43`) | `StarlarkBuildSettingsDetailsValue` `starlark_transition.rs:50` | derived (`:49`) | `J:.../analysis/starlark/StarlarkBuildSettingsDetailsValue.java:96` `record Key` | `J:.../analysis/starlark/StarlarkBuildSettingsDetailsValue.java:55` **record** (4 immutable maps and sets) | Hermetic, `SkyFunctions.java:117-118` | `EQ-OK` (prelim.). `IndexMap`/`IndexSet` `==` ignores order, like `ImmutableMap`/`ImmutableSet`. `Type` is a `Copy` enum (`crates/blaze_packages/src/types/type_.rs:51`); Java `Type` has no `equals` but its instances are singletons. Defaults: Java uses `Object.equals`, Rust uses `StarlarkOptionValue` derived `==` (`crates/blaze_build_config/src/build_options.rs:206`). The doc comment at :204-205 says `==` is structural and stricter than Java across collection classes (`ImmutableList` vs `ArrayList`), so again it can only miss a prune. S4: the Rust `// Java:` marker at `:48` names `skyframe/StarlarkBuildSettingsDetailsValue.java:38`; the class is in `analysis/starlark/` at :55 |
| 5 | `crates/blaze_analysis/src/aspect_key_creator.rs:371` `AspectKey` | `ASPECT` (`aspect_key_creator.rs:31`) | `AspectValue` `crates/blaze_analysis/src/aspect_value.rs:21` | none | `J:.../skyframe/AspectKeyCreator.java:101` | `J:.../analysis/AspectValue.java:34` `extends BasicActionLookupValue implements ConfiguredAspect, RuleConfiguredObjectValue`. `RuleConfiguredObjectValue` (`analysis/RuleConfiguredObjectValue.java:23`) extends `ConfiguredObjectValue`, which extends `NotComparableSkyValue` (`ConfiguredObjectValue.java:24`). Subclasses at :143, :184, :199 add no equals | Hermetic, `SkyFunctions.java:101` | `IDENTITY` (prelim.): NotComparable |
| 6 | `aspect_key_creator.rs:524` `TopLevelAspectsKey` | `TOP_LEVEL_ASPECTS` (`aspect_key_creator.rs:35`; a duplicate string const is at `crates/blaze_sky_exec/src/keys.rs:260`) | `TopLevelAspectsValue` `aspect_key_creator.rs:572` | none (`Clone, Debug` :571) | `J:.../skyframe/AspectKeyCreator.java:313` | `J:.../skyframe/TopLevelAspectsValue.java:30` `implements ActionLookupValue` (a plain SkyValue); no equals: identity | Hermetic, `SkyFunctions.java:102-103` | `IDENTITY` (prelim.): no equals, and it wraps NotComparable `AspectValue`s |
| 7 | `aspect_key_creator.rs:670` `LoadAspectsKey` | `LOAD_ASPECTS` (`aspect_key_creator.rs:39`) | `LoadAspectsValue` `aspect_key_creator.rs:678` | none (`Clone` :677, manual `Debug`) | `J:.../skyframe/LoadAspectsKey.java:32` | `J:.../skyframe/LoadAspectsValue.java:25` `implements SkyValue`; no equals: identity | Hermetic, `SkyFunctions.java:104` | `IDENTITY` (prelim.): no equals |
| 8 | `crates/blaze_sky_analysis/src/toolchains/registered_execution_platforms.rs:95` `RegisteredExecutionPlatformsKey` | `REGISTERED_EXECUTION_PLATFORMS` (`:37`) | `RegisteredExecutionPlatformsValue` `:52` | derived (`:51`) | `J:.../skyframe/toolchains/RegisteredExecutionPlatformsValue.java:58` `Key` (equals :93) | `J:.../skyframe/toolchains/RegisteredExecutionPlatformsValue.java:42` **record** (`ImmutableList<ConfiguredTargetKey>`, `@Nullable ImmutableMap<Label,String>`) | Hermetic, `SkyFunctions.java:139-140` | `EQ-OK` (prelim.). An ordered `Vec` matches `ImmutableList`. `Option<IndexMap>` ignores order like `ImmutableMap`. The derived `ConfiguredTargetKey` eq (`configured_target_key.rs:26`) uses the same 4 fields as `ConfiguredTargetKey.java:127-137` |
| 9 | `crates/blaze_sky_analysis/src/toolchains/single_toolchain_resolution.rs:136` `SingleToolchainResolutionKey` | `SINGLE_TOOLCHAIN_RESOLUTION` (`:29`) | `SingleToolchainResolutionValue` `:41` | derived (`:40`) | `J:.../skyframe/toolchains/SingleToolchainResolutionValue.java:89` (`@AutoValue`) | `J:.../skyframe/toolchains/SingleToolchainResolutionValue.java:46` **record** (`ToolchainTypeInfo`, `ImmutableMap<ConfiguredTargetKey,Label>`) | Hermetic, `SkyFunctions.java:143-144` | `EQ-OK` (prelim.). The derived `ToolchainTypeInfo` eq (`crates/blaze_analysis/src/platform/toolchain_type_info.rs:24`; fields `type_label`, `none_found_error`) equals `J:.../analysis/platform/ToolchainTypeInfo.java:81-87`. `IndexMap` `==` ignores order |
| 10 | `crates/blaze_sky_analysis/src/toolchains/registered_toolchains.rs:92` `RegisteredToolchainsKey` | `REGISTERED_TOOLCHAINS` (`:35`) | `RegisteredToolchainsValue` `:49` | derived (`:48`) | `J:.../skyframe/toolchains/RegisteredToolchainsValue.java:57` `Key` (equals :102) | `J:.../skyframe/toolchains/RegisteredToolchainsValue.java:42` **record** (`ImmutableList<DeclaredToolchainInfo>`, `@Nullable ImmutableTable<Label,Label,String>`) | Hermetic, `SkyFunctions.java:141-142` | `EQ-OK` (prelim.). The manual `DeclaredToolchainInfo::eq` (`crates/blaze_analysis/src/platform/declared_toolchain_info.rs:86`) covers the 6 record components and compares `Arc<ConstraintCollection>` by value. Table: Rust nests `IndexMap<Label, IndexMap<Label,String>>`. It equals Java's cell-set equality because rows are only created on insert (`registered_toolchains.rs:203-207`), so there are no empty rows |

Counts: 10 keys, 10 distinct value types. `IDENTITY` 5 (rows 1, 2, 5, 6, 7). `EQ-OK` 5 (rows 3, 4, 8, 9, 10; rows 3 and 4 are stricter than Java only in collection order or class, which is safe). `EQ-WRITE` 0, `RUST-STRONGER` 0, `MIXED` 0. All 10 Java function names are `createHermetic`.

### B. Inputs that are not dependencies (G1)

Registration: `crates/blaze_sky_top/src/skyframe_executor.rs:341-466` builds one `AnalysisServices` per executor, which is per command today (`:354-382`). It then registers:
- toolchain functions (`crates/blaze_sky_analysis/src/toolchains/mod.rs:44-61`: plain fns, `_services` unused, nothing captured);
- `ConfiguredTargetFunction{services, store_transitive_packages: true}` (`configured_target_function.rs:75-87`, which also registers STARLARK_BUILD_SETTINGS_DETAILS with no capture);
- `AspectFunction{services, ..}` (`aspect_function.rs:105-116`);
- `ToplevelStarlarkAspectFunction{services, ..}` (`toplevel_starlark_aspect_function.rs:60-71`);
- `LoadAspectsFunction` (unit, `load_aspects_function.rs:60`);
- BUILD_INFO, a closure over `services` (`skyframe_executor.rs:442-446`);
- PARSED_FLAGS, which captures the `options_classes` vec, a server constant (`crates/blaze_sky_analysis/src/parsed_flags_function.rs:32-37`);
- `PlatformFunction{services}`, which stores `services` but never reads it (`crates/blaze_sky_analysis/src/platform/platform_function.rs:40-45`).

`env.client::<T>()` readers in these crates: none (grep).

`AnalysisServices` fields (`crates/blaze_analysis/src/analysis_services.rs:39-66`):

| Field (`analysis_services.rs`) | Read inside a function at | Per-command? | Java | Class |
|---|---|---|---|---|
| `rule_class_provider` :40 | `configured_target_function.rs:469`, `aspect_function.rs:794` | rebuilt per executor (`skyframe_executor.rs:355-357`) | server constant (`ConfiguredRuleClassProvider`) | (a) |
| `bazel_starlark_environment` :42 | no reader in these crates | built per executor | `ruleClassProvider.getBazelStarlarkEnvironment()`, a server constant | (a) |
| `artifact_factory` :44 | `configured_target_function.rs:436, 472`, `aspect_function.rs:762`, `analysis_services.rs:151, 156` | new per executor (`skyframe_executor.rs:360-363`) | `SkyframeBuildView.artifactFactory` (`J:.../skyframe/SkyframeBuildView.java:141`), server lifetime; `artifactFactory.clear()` in `clearLegacyData` (`SkyframeBuildView.java:1418`), called from `handleAnalysisInvalidatingChange` (`SkyframeExecutor.java:1222`) and `reset` (:1430) | (c) |
| `action_key_context` :45 | `configured_target_function.rs:437`, `aspect_function.rs:763` | new per executor | `SkyframeBuildView.actionKeyContext` (`SkyframeBuildView.java:143`), server lifetime; `actionKeyContext.clear()` after every command (`J:.../runtime/BlazeRuntime.java:833`) | (c) |
| `starlark_semantics` :47 | `crates/blaze_sky_analysis/src/producers/transition_applier.rs:115, 152`; `producers/target_and_configuration_producer.rs:570`; `dependency_resolver.rs:683` | yes, from options (`skyframe_executor.rs:226, 366`) | `PrecomputedValue.STARLARK_SEMANTICS` (`J:.../skyframe/PrecomputedValue.java:85`). In Java the transition carries the semantics of its `.bzl` load (`analysis/config/StarlarkDefinedConfigTransition.java:381`), so it arrives through deps (BZL_LOAD, which depends on STARLARK_SEMANTICS) | (b) |
| `directories` :48 | `analysis_services.rs:141-143` | from inputs | `BlazeDirectories`, a server constant | (a) |
| `main_repo_mapping: RwLock<Option<..>>` :50 | `configured_target_function.rs:444`, `aspect_function.rs:770`; written by `skyframe_executor.rs:746-747` | yes | **Java requests it as a dependency**: `skyframeEnv.getValue(RepositoryMappingValue.key(MAIN))` in `J:.../analysis/CachingAnalysisEnvironment.java:368-377`. The toolchain functions already do this (`registered_toolchains.rs:121`, `registered_execution_platforms.rs:111`) | Not (a)-(d): a **missing dependency edge**. It must become `env.get_value(RepositoryMappingKey::key(MAIN))`. With a kept graph, a changed main repo mapping leaves CT and aspect nodes stale. S1 hazard |
| `keep_going` :52 | no reader in these crates (grep `keep_going` in both crates: only the field) | yes (`skyframe_executor.rs:375`) | per evaluation | (d), and unused in this group |
| `workspace_status: OnceLock` :55 | `configured_target_function.rs:446`, `aspect_function.rs:772` (through `get_workspace_status_artifacts`, `analysis_services.rs:136-174`), and the BUILD_INFO function (`analysis_services.rs:109`) | once per services object | **dependency** on `WorkspaceStatusValue.BUILD_INFO_KEY` (`CachingAnalysisEnvironment.java:357-363`); the node is Hermetic and has no deps (`WorkspaceStatusFunction.java:38-50`) | **Missing dependency edge** (marked `// PORT:` at `configured_target_function.rs:445`). The fix is `env.get_value(WorkspaceStatusKey)`, after which the OnceLock goes away; the BUILD_INFO function then makes the artifacts itself, as Java does |
| `workspace_status_user_name`, `workspace_status_host_name` :58-59 | `analysis_services.rs:116-117` (BUILD_INFO) | computed per executor (`skyframe_executor.rs:379-380`) | `USER_NAME.value()` and `NetUtil.getCachedShortHostName()` (`J:.../bazel/BazelWorkspaceStatusModule.java:283`), process statics | (a) |
| `base_target_prerequisites_supplier: OnceLock` :63 | `aspect_function.rs:216` and through it `dependency_map_producer.rs:387, 862`, `configured_target_and_data_producer.rs:61, 120` | set once after the evaluator is built (`skyframe_executor.rs:471-485`) | `SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl` (`J:.../skyframe/SkyframeExecutor.java:4598`). Java also calls `memoizingEvaluator.getExistingValue` outside the dependency mechanism | (a); Java does the same |

Other captured or static inputs read inside functions in these crates:

| Site | Content | Java | Class |
|---|---|---|---|
| `crates/blaze_analysis/src/config/starlark_transition.rs:622` `StarlarkTransitionCache::get()`, used by `transition_applier.rs:109` | transition results cache | `SkyframeBuildView.starlarkTransitionCache` (`SkyframeBuildView.java:174`) | (c): see E/G6 |
| `crates/blaze_sky_analysis/src/dependency_resolver.rs:666` `LOADED` | exec transition provider | BZL_LOAD dependency | see E/G6 |
| `crates/blaze_sky_analysis/src/analysis_progress_receiver.rs:20` `ANALYSIS_PROGRESS` (written from `configured_target_function.rs:297`, `aspect_function.rs:711, 908`) | progress counters | `SkyframeBuildView` field; Java increments in compute (`ConfiguredTargetFunction.java:385`, `AspectFunction.java:923, 1038`) | (c): reset by `crates/blaze_sky_top/src/build_tool.rs:516`. Output only |

### C. File system reads in function bodies (G2)

| Site | What | Java | Verdict |
|---|---|---|---|
| `crates/blaze_analysis/src/rule/rule_context.rs:2118` `prerequisite.get_input_path().is_some_and(\|p\| p.exists())` | stats an input file's path during `validateDirectPrerequisiteType` (CONFIGURED_TARGET) | `J:.../analysis/RuleContext.java:1869` `!prerequisite.getInputPath().exists()`: Java does the same direct stat, with no FileValue | allowed (Java does the same). It is an incrementality hole in Bazel too: it only chooses an error message |
| `crates/blaze_sky_analysis/src/producers/dependency_map_producer.rs:1323` `std::fs::read_to_string` | inside `#[cfg(test)] mod tests` (:1316) | n/a | test only, excluded |
| `crates/blaze_build_config/src/starlark_transition.rs:866` `load_starlark_exec_transition`, called from `dependency_resolver.rs:683` | evaluates the bundled `@_builtins//:common/builtin_exec_platforms.bzl` from compiled-in text (`load_builtins_bzl` :937+); **no file read** | Java loads it through BZL_LOAD (`J:.../analysis/config/StarlarkExecTransitionLoader.java:78-93`) | No G2 read today. It is a missing BZL_LOAD dependency and only matters once `--experimental_builtins_bzl_path` or a user exec config file is ported (both `unported!` at `starlark_transition.rs:848-853, 941-946`) |

The `is_file()` / `is_directory()` hits (`configured_target_and_data_ext.rs:51`, `starlark_action_factory.rs:1496-1502`) are `Target` and `Artifact` kind checks, not file system calls.

### D. Interior mutability reachable from this group's values (G4)

| Site | Reached from | Memo or mutation | Note |
|---|---|---|---|
| `crates/blaze_actions/src/actions/artifact.rs:66` `generating_action_key: OnceLock<ActionLookupData>`; setter at :375 | every `Artifact` in `ConfiguredTargetValue.actions` and providers, `AspectValue`, `WorkspaceStatusValue` | Set before the value is returned. CT: `crates/blaze_analysis/src/rule/rule_configured_target_builder.rs:308` → `rule/actions_util.rs:215`. Aspect: `configured_aspect.rs:252`. BUILD_INFO: `analysis_services.rs:166-168`, inside the `workspace_status` OnceLock init, which may run first from a CT function (before BUILD_INFO runs) | Java does the same (`DerivedArtifact.setGeneratingActionKey`, `ActionsUtil.assignOwnersAndThrowIfConflict`, `WorkspaceStatusFunction.java:44-47`). Set-once, before publication: fine. With a kept graph and per-command `AnalysisServices`, a fresh services object makes new workspace status `Artifact` objects, which equal the old ones by value (owner and exec path). Other setters outside this group: `blaze_sky_top/src/action_template_expansion_function.rs:262`; the `blaze_sky_exec` hits are tests |
| `crates/blaze_analysis/src/configuredtargets/mod.rs:110, 384, 452, 551, 609` `default_info: OnceLock<OwnedFrozenValue>` | `ConfiguredTarget` (Rule, InputFile, OutputFile, PackageGroup, Alias...) in `ConfiguredTargetValue` | memo (lazy DefaultInfo Starlark value) | fine |
| `crates/blaze_analysis/src/test/test_runner_action.rs:132` `execution: OnceLock<Arc<TestRunnerAction>>` | `ConfiguredTargetValue.actions` | memo ("built on first use from the fields above") | fine |
| `crates/blaze_collect/src/nestedset/nested_set.rs:105` `cached: Option<Mutex<Weak<Vec<E>>>>` | `transitive_packages`, providers | memo (`toList` cache) | fine |
| `crates/blaze_analysis/src/analysis_services.rs:50, 55, 63` (`RwLock`, 2x `OnceLock`) | not inside a value; captured services | `main_repo_mapping` is rewritten per command (`skyframe_executor.rs:747`); the OnceLocks are set once per services object | See B: two become deps, one stays |
| `ConfiguredTargetValue.target: Option<Target>` (`configured_target_value.rs:30`) → `Package` | CT value | the `PackageDeclarations` `RwLock`/`AtomicBool` named in research 2.5 G4 (`crates/blaze_packages/src/package/package.rs:227-235`) | Not this group; for the `blaze_packages` audit |
| `UnloadedToolchainContextImpl`, `RegisteredToolchainsValue`, `RegisteredExecutionPlatformsValue`, `SingleToolchainResolutionValue`, `StarlarkBuildSettingsDetailsValue`, `LoadAspectsValue` | n/a | none found one level down (`Arc<PlatformInfo>`, `ConstraintCollection`, `Label`, `IndexMap`) | fine |

`RuleContext`, `CachingAnalysisEnvironment` (`rule/analysis_environment.rs:72, 78`), `StarlarkRuleContext` `Cell`s and `StarlarkActionFactory` atomics are per-evaluation builder state. None is reachable from a returned value (`ConfiguredTargetBase` holds only `lookup_key` and `visibility`, `configured_target.rs:22-30`).

### E. Events, caches, side effects (G5, G6, G7)

#### G5: output not through `env.listener()`
- `crates/blaze_analysis/src/starlark/starlark_action_factory.rs:783-788` `system_err_println`, writing to stderr; called at :772 from `ResourceSetCallbackEvents::debug` (`print()` inside a `resource_set` callback at **execution** time). Java: `J:net/starlark/java/eval/StarlarkThread.java:366` `defaultPrintHandler` → `System.err`. Allowed (Java does the same), and not inside a SkyFunction compute.
- No `eprintln!`/`println!` in either crate. `blaze_sky_analysis` reports through `env.listener()` (34 sites). No captured reporter in these registrations.
- `StarlarkTransitionCache` stores the non-error events of a transition and replays them on a hit (`starlark_transition.rs:722-726`), as Java does (`StarlarkTransitionCache.java:134+`).

#### G6: caches outside the graph consulted inside functions

| Site | Keyed by | Java | Problem with a kept graph |
|---|---|---|---|
| `crates/blaze_analysis/src/config/starlark_transition.rs:622` `INSTANCE: LazyLock<StarlarkTransitionCache>` (maps at :603, :605); `clear()` at :772 has **no caller** (grep `StarlarkTransitionCache` / `.clear()`) | transition `cache_key()` (for Starlark transitions: the `Arc` **address** of the definition, `crates/blaze_analysis/src/config/transitions.rs:324-330`), from-options checksum, `details` Debug text | Per `SkyframeBuildView` (`SkyframeBuildView.java:174`), cleared at **:392** (`clearAnalysisCache`, `--discard_analysis_cache`), **:1419** (`clearLegacyData`, from `handleAnalysisInvalidatingChange` `SkyframeExecutor.java:1222` and `reset` :1430) and **:1444** (`clearInvalidatedActionLookupKeys`, called after every analysis, `J:.../analysis/BuildView.java:437`). All three confirmed | Process-wide and never cleared. After a `.bzl` edit the old definition `Arc` stays pinned by `CONVERTED`, so a new definition gets a new address and stale hits are unlikely, but the map grows without bound. Semantics are not in the key; Java relies on the per-build clear for that. Port the `:1444` clear (after each analysis) and the `:1419` clear |
| `crates/blaze_sky_analysis/src/dependency_resolver.rs:666` `LOADED: LazyLock<Mutex<HashMap<String, Option<Arc<..>>>>>` | `--experimental_exec_config` text only | BZL_LOAD dependency (`StarlarkExecTransitionLoader.java:78-93`), memoized by Skyframe | Ignores `starlark_semantics` (`load_starlark_exec_transition(.., &services.starlark_semantics)` at :683). A semantics change between commands returns a provider built under the old semantics. The bundled file is compile-time text, so no `.bzl` edit staleness today (the research doc's "stale after the .bzl is edited" applies once user or `--experimental_builtins_bzl_path` files are ported). Reset per command, or make it a dep |
| `crates/blaze_analysis/src/config/transitions.rs:897` `CONVERTED: LazyLock<Mutex<Vec<(String, Arc<..>)>>>` | `format!("{:?}", frozen.identity())`: a frozen value **address** | Java keeps one object per `transition()` value; no cache | Correct only while every `.bzl` heap is kept forever (relies on `blaze_sky_loading` `KEEP_ALIVE`, `bzl_load.rs:732`). If the heaps are ever released, a reused address returns the wrong transition. Linear scan; unbounded |
| `crates/blaze_analysis/src/rule/depsets.rs:70` `MEMO`, `:74` `FROZEN_TO_SET`, `:122` `KEPT_HEAPS` (keeper at :125-128, called from `depsets.rs:599, 674`, `starlark/starlark_aspect_factory.rs:172`, `starlark/starlark_rule_configured_target_util.rs:238`) | (TypeId, node address) | none (Java's depset is the NestedSet) | Correct because sources and heaps are kept alive forever; memory is unbounded across commands. Dropping `FROZEN_TO_SET` while kept CT values still hold the converted `NestedSet`s would make a re-converted depset a different `NestedSet` identity. **Needs a decision**: keep, or evict together with the graph nodes that own the heaps |
| `crates/blaze_analysis/src/starlark/fragment_collection.rs:41` `FRAGMENT_FACTORIES`; `starlark_action_factory.rs:2090` `TRANSFORM_BUILD_INFO_FILE`; `rule/rule_context.rs:104` `ATTR` | hook registration and constants | server constants | fine |

#### G7: side effects that must be repeatable
None in either crate. Greps for `Command::new`, `fs::write`, `create_dir`, `File::create`, `fs::remove`, `SystemTime::now`, `Instant::now` and `env::var` found no hit outside tests, apart from the stderr write above. The progress counters (`analysis_progress_receiver.rs:20`) are output only and are reset per build (`blaze_sky_top/src/build_tool.rs:516`). Java counts in compute too, so a clean node is not counted in either.

### Surprises
1. Two **missing dependency edges** in CONFIGURED_TARGET and ASPECT: `main_repo_mapping` (an `RwLock` the executor writes) and the workspace status artifacts (`OnceLock`). Java requests both as Skyframe deps (`CachingAnalysisEnvironment.java:357, 368`). The first is an S1 hazard once the graph is kept.
2. `StarlarkTransitionCache::clear()` is dead code; Java clears it after every analysis (`SkyframeBuildView.java:1444` via `BuildView.java:437`).
3. `LOADED` reads no file (the exec transition `.bzl` is compiled in), but its key leaves out semantics.
4. No `RUST-STRONGER` or `EQ-WRITE` in this group. The two "EQ-OK, stricter" rows (3, 4) compare collection order or Java class where Java does not, so they can only miss prunes.

### Greps run
- `grep -rn 'impl.*Key for\|impl Key for\|impl blaze_skyframe::Key' crates/blaze_analysis/src crates/blaze_sky_analysis/src`
- `grep -rn 'impl PartialEq for \(<each value type>\)' crates`; `grep -rn -B3 'pub struct <field types>'` (PlatformInfo, ToolchainTypeInfo, NoMatchingPlatformData, ToolchainTypeRequirement, StarlarkOptionValue, DeclaredToolchainInfo, ToolchainContextKey, ConfigMatchingProvider, PlatformProperties)
- Java: `grep -n 'class \|interface \|record \|equals\|hashCode\|NotComparable\|@AutoValue'` over each value class and its superclasses/interfaces; `grep -n` in `SkyFunctions.java` for each name; `SkyFunctionName.java` `create*`
- `grep -rnE '^\s*(pub(\([^)]*\))?\s+)?static\s+[A-Za-z_]\w*\s*:|thread_local!'` over both crates
- `grep -rnE '<field>:\s*.*(OnceLock|OnceCell|Mutex|RwLock|Atomic..|RefCell|Cell<)'` over both crates
- `grep -rnE 'std::fs|fs::(read|metadata|...)|read_to_string|\.exists\(\)|\.is_file\(\)|\.is_dir\(\)|\.stat\(|readlink|get_input_stream|\.metadata\(|File::open|...'` over both crates
- `grep -rnE '\.(get_digest|get_file_size|get_last_modified_time|resolve_symbolic_links|read_symbolic_link|get_fast_digest|stat_nullable|stat_if_found|get_directory_entries|readdir|read_lines|read_content|get_input_stream)\('`
- `grep -rnE 'std::process::Command|Command::new|fs::write|create_dir|fs::remove|File::create|\.write_all\('`; `grep -rnE 'SystemTime::now|Instant::now|env::var\('`
- `grep -rnE 'eprintln!|println!|eprint!|print!\(|std::io::stderr|std::io::stdout'`; `grep -rn 'env\.listener()'` (34 in `blaze_sky_analysis`)
- `grep -rn 'services\.' crates/blaze_sky_analysis/src` (field readers); `grep -rn 'keep_going'` both crates
- `grep -rn 'StarlarkTransitionCache'`, `.clear()` callers; Java `grep -n 'starlarkTransitionCache' SkyframeBuildView.java`, the callers of `clearLegacyData`/`clearInvalidatedActionLookupKeys`/`clearAnalysisCache`
- `grep -rn 'set_generating_action_key'`, `assign_owners_and_throw_if_conflict` callers
- Java `grep -rn 'actionKeyContext.clear\|artifactFactory.clear'`, `BUILD_INFO_KEY` uses, `BazelWorkspaceStatusModule.java` user and host name

### Not checked
- `ConfigMatchingProvider` derived `PartialEq` (`crates/blaze_analysis/src/config/config_matching_provider.rs:107`) against Java `@AutoValue` field by field (it reaches rows 3 and 10 through `PlatformInfo.required_settings` and `DeclaredToolchainInfo.target_settings`).
- `ConstraintCollection::eq` (`crates/blaze_analysis/src/platform/constraint_collection.rs:272`) and `PlatformProperties` derived eq against Java.
- Whether `ArtifactFactory` (`blaze_actions`) holds its own cache that needs Java's `clear()`; the owner is `blaze_actions`.
- Interior mutability deeper than two levels: Starlark provider values in `TransitiveInfoProviderMap`, and `Aspect`/`AspectDefinition` in `blaze_packages`.
- `Package` mutation after return (belongs to the `blaze_packages` group).
- Helpers in other crates (`blaze_rules` rule implementations) that CONFIGURED_TARGET calls through `rule_class_provider`: not scanned for file system reads or statics.

### F. Group 4 worker: files, order, hand-back

Files:
- `crates/blaze_analysis/src/{configured_target_key.rs, analysis_services.rs, aspect_key_creator.rs, configured_target_value.rs, aspect_value.rs}`
- `crates/blaze_analysis/src/toolchains/toolchain_context_key.rs`
- `crates/blaze_analysis/src/config/starlark_transition.rs`
- `crates/blaze_sky_analysis/src/toolchains/{registered_execution_platforms.rs, registered_toolchains.rs, single_toolchain_resolution.rs}`
- `crates/blaze_sky_analysis/src/{configured_target_function.rs, aspect_function.rs, dependency_resolver.rs}`
- `crates/blaze_sky_analysis/src/producers/{transition_applier.rs, target_and_configuration_producer.rs}`
- `crates/blaze_sky_analysis/src/platform/platform_function.rs` (shared with group 3, step 3)

Order:
1. `HERMETICITY` on the 10 keys (all Hermetic).
2. Opt in the 5 EQ-OK keys. Optionally make the `Vec`-as-set fields of `UnloadedToolchainContextImpl`
   compare as sets.
3. **Missing dependency edges** (S1 hazards once the graph is kept). These come first among the G1
   work:
   - `main_repo_mapping` (`analysis_services.rs:50`, read at `configured_target_function.rs:444`,
     `aspect_function.rs:770`) becomes `env.get_value(RepositoryMappingKey(MAIN))`, as in
     `J:com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:368-377`;
   - the workspace status artifacts (the `OnceLock` at `analysis_services.rs:55`) become a dependency
     on `WorkspaceStatusKey` (`CachingAnalysisEnvironment.java:357-363`).
4. G1, after X1: `starlark_semantics` becomes `STARLARK_SEMANTICS` (readers at
   `transition_applier.rs:115, 152`, `target_and_configuration_producer.rs:570`,
   `dependency_resolver.rs:683`). `artifact_factory` and `action_key_context` become server-lifetime
   objects with Java's `clear()` points (`SkyframeBuildView.java:1418`, `BlazeRuntime.java:833`); the
   clear calls themselves are I5's. `keep_going` is unused here and can be dropped from
   `AnalysisServices` or moved to the evaluation context.
5. G6:
   - `StarlarkTransitionCache::clear()` after each analysis and at `clearLegacyData`
     (`SkyframeBuildView.java:1419, 1444`): the call sites live in `blaze_sky_top` and are requested
     from group 6;
   - `LOADED` (`dependency_resolver.rs:666`): key it by semantics as well, or reset it per command;
   - `CONVERTED` and the `depsets.rs` memos: record the decision (X7).

Hand back: one line per key. Per `AnalysisServices` field: its class and what was done. Tests:
- a change of the main repository mapping (a `MODULE.bazel` `repo_name` change) must dirty
  CONFIGURED_TARGET;
- the toolchain values must prune on recompute.

## Group 5: blaze_sky_exec (9 keys)

Helper draft: **G5: `blaze_sky_exec` (I2 survey draft)**

Tree: HEAD `4f92912` plus uncommitted work of others (line numbers as of 2026-11-19). Read-only; no Cargo run.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`. Rust paths are relative to `crates/blaze_sky_exec/src/` unless they start with `crates/`.

### A. Keys and values (9 production `impl Key`)

| # | Rust key | FUNCTION | Rust value type | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `keys.rs:31` `ActionExecutionKey` | `ACTION_EXECUTION` | `ActionExecutionValue` `action_execution_value.rs:19` | none (`#[derive(Clone, Debug, Default)]` `:18`) | `J:com/google/devtools/build/lib/actions/ActionLookupData.java:33` | `J:com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:59`, `equals` at line 232: `getAllFileValues().equals(..)` (ImmutableMap, order-insensitive, values are `FileArtifactValue.equals`), `getAllTreeArtifactValues().equals(..)` (`TreeArtifactValue.equals`), `Objects.equals(getRichArtifactData(), ..)` (`RunfilesArtifactValue.equals` `J:.../actions/RunfilesArtifactValue.java:169`), `getDiscoveredModules().shallowEquals(..)` (comment :242-243: "potentially less effective, but never incorrect") | SemiHermetic `J:.../skyframe/SkyFunctions.java:120-121` | **EQ-WRITE** (prelim.): blockers: (1) `TreeArtifactValue` (`crates/blaze_exec/src/actions/input_metadata_provider.rs:91`) has no `PartialEq`; Java `TreeArtifactValue.equals` `J:.../skyframe/TreeArtifactValue.java:399` compares digest, childData, archivedRepresentation, resolvedPath; Rust fields `child_values`, `archived_representation`, `metadata`, `resolved_path` all have `PartialEq` (`ActionInputRef` manual eq `crates/blaze_exec/src/actions/action_input.rs:164`: ptr or kind+exec path, weaker than Java `Artifact.equals`, PORT-noted), so derivable. (2) `RunfilesArtifactValue` (`input_metadata_provider.rs:353`) has no `PartialEq`; it holds `runfiles_tree: Arc<dyn RunfilesTree>`, which Java's `equals` deliberately ignores (comment `RunfilesArtifactValue.java:170-175`), so write it manually over `metadata, files, file_values, trees, tree_values` (filesets absent, PORT-noted). (3) No discovered-modules field in the Rust value (Java `getDiscoveredModules()` :219); C++ module discovery would need to be carried before opt-in or proved absent. `rich_artifact_metadata` duplicates `RunfilesArtifactValue.metadata`; must not be compared separately in a way that diverges. `IndexMap` `PartialEq` is order-insensitive, like `ImmutableMap`. |
| 2 | `keys.rs:46` `ArtifactKey` | `ARTIFACT` | `ArtifactValue` enum `artifact_function.rs:34` (`File(FileArtifactValue)` / `Missing(MissingArtifactValue)` / `Tree(Arc<TreeArtifactValue>)`) | none (`#[derive(Clone, Debug)]` `:33`) | `J:.../actions/Artifact.java:119` (sealed abstract class; itself the key) | `ArtifactFunction.compute` (`J:.../skyframe/ArtifactFunction.java:131`) returns: `FileArtifactValue` (`J:.../actions/FileArtifactValue.java:84`; subclass `equals` at 470 Directory(mtime), 532 HashedDirectory, 593 RegularFile(digest, proxy, size), 722 RunfilesProxy, 756/903 Remote, 1063 ResolvedSymlink, 1196 UnresolvedSymlink(digest only), 1271 Inline, 1311 Proxy; 1431 ConstantMetadataValue singleton); `TreeArtifactValue` (`equals` at `TreeArtifactValue.java:399`); `MissingArtifactValue` (`ArtifactFunction.java:80`, no equals: identity) | Hermetic `J:.../actions/Artifact.java:159` (comment :150-158: source artifacts make it effectively non-hermetic; TODO) | **EQ-WRITE** (prelim.): needs a manual `values_equal`: `File`/`File` by `FileArtifactValue` derived `PartialEq` (`crates/blaze_exec/src/actions/file_artifact_value.rs:116`; agrees with Java per variant; `UnresolvedSymlink` also compares `target`, equivalent since digest is of the target; `RemoteFile` symmetric vs Java's asymmetric class check, PORT-noted at :133); `Tree`/`Tree` needs `TreeArtifactValue: PartialEq` (row 1); `Missing` must stay never-equal (identity in Java). |
| 3 | `keys.rs:124` `ArtifactNestedSetKey` | `ARTIFACT_NESTED_SET` | `ArtifactNestedSetValue` enum `keys.rs:117` | derived `keys.rs:115` | `J:.../collect/nestedset/ArtifactNestedSetKey.java:45` (Rust comment `keys.rs:64` cites `skyframe/ArtifactNestedSetKey.java:39`: stale path, S4) | `J:.../skyframe/ArtifactNestedSetValue.java:27` enum `implements NotComparableSkyValue` (line 27); comment line 23: "Implements {@link NotComparableSkyValue} to prohibit value-based change pruning." | Hermetic `SkyFunctions.java:122-123` | **RUST-STRONGER** (prelim.): derived `PartialEq` on the enum must not be used for pruning. |
| 4 | `keys.rs:152` `TargetCompletionKey` | `TARGET_COMPLETION` | `TargetCompletionValue` unit struct `completion_function.rs:25` | none (`#[derive(Clone, Debug, Default)]` `:24`) | `J:.../skyframe/TargetCompletionValue.java:53` (`@AutoValue`) | `TargetCompletionValue.java:28`, no equals (identity), but a singleton: `INSTANCE` :29, the only value returned (`J:.../skyframe/TargetCompletor.java:82`), so every two values are equal | Hermetic `SkyFunctions.java:105-106` | **EQ-WRITE** (prelim.): trivial: derive `PartialEq` on the unit struct (or `values_equal` = true). |
| 5 | `keys.rs:191` `AspectCompletionKey` | `ASPECT_COMPLETION` | `AspectCompletionValue` unit struct `completion_function.rs:685` | none (`:684`) | `J:.../skyframe/AspectCompletionValue.java:41` (`@AutoValue`) | `AspectCompletionValue.java:30`, no equals, singleton `INSTANCE` :31, returned at `J:.../skyframe/AspectCompletor.java:86` | Hermetic `SkyFunctions.java:107-108` | **EQ-WRITE** (prelim.): trivial, as row 4. |
| 6 | `keys.rs:218` `TestCompletionKey` | `TEST_COMPLETION` | `TestCompletionValue` unit struct `test_completion_function.rs:21` | none (`:20`) | `J:.../skyframe/TestCompletionValue.java:59` (`@AutoValue`) | `TestCompletionValue.java:31`, no equals, singleton `TEST_COMPLETION_MARKER` :32, returned at `J:.../skyframe/TestCompletionFunction.java:96` | Hermetic `SkyFunctions.java:109` | **EQ-WRITE** (prelim.): trivial, as row 4. |
| 7 | `keys.rs:310` `BuildDriverKey` | `BUILD_DRIVER` | `BuildDriverValue` `build_driver_function.rs:45` (holds `TopLevelSkyValue` = `Arc<dyn ConfiguredTargetView>` / `Vec<Arc<dyn ConfiguredAspectView>>` `:20-25`) | none (`#[derive(Clone, Debug)]` `:44`) | `J:.../skyframe/BuildDriverKey.java:26` | `J:.../skyframe/BuildDriverValue.java:19` no equals: identity; subclass `ExclusiveTestBuildDriverValue.java:20` no equals | NonHermetic `SkyFunctions.java:154-155` | **IDENTITY** (prelim.): leave default. |
| 8 | `action_template_expansion_value.rs:142` `ActionTemplateExpansionKey` | `ACTION_TEMPLATE_EXPANSION` (const from `blaze_actions::actions::action_lookup`) | `ActionTemplateExpansionValue` `action_template_expansion_value.rs:21` (holds `Vec<ActionRef>`) | none (no derive) | `J:.../skyframe/ActionTemplateExpansionValue.java:64` (manual equals :124) | `ActionTemplateExpansionValue.java:33` extends `J:.../actions/BasicActionLookupValue.java:25`; neither has equals: identity | Hermetic `SkyFunctions.java:135-136` | **IDENTITY** (prelim.): leave default. Function lives in `crates/blaze_sky_top/src/action_template_expansion_function.rs:42` (other group). |
| 9 | `recursive_filesystem_traversal_function.rs:250` `DirectoryArtifactTraversalRequest` | `RECURSIVE_FILESYSTEM_TRAVERSAL` (const `:60`) | `RecursiveFilesystemTraversalValue` `recursive_filesystem_traversal_function.rs:448` | derived `:447` (`ResolvedFile`, `HasDigest` derived; `NestedSet` eq is `Arc::ptr_eq`, `crates/blaze_collect/src/nestedset/nested_set.rs:153`) | `J:.../skyframe/ArtifactFunction.java:570` (equals :640 ignores `artifact`; Rust `:217` same) | `J:.../skyframe/RecursiveFilesystemTraversalValue.java:51`, `equals` at 621: `resolvedRoot.equals && resolvedPaths.equals`; `NestedSet` has no `equals` (identity; only `shallowEquals` `J:.../collect/nestedset/NestedSet.java:532`). `ResolvedFile` subclasses' equals at 305, 355 (path only), 407, 465, 524 | Hermetic `SkyFunctions.java:124-125` | **EQ-OK** (prelim.): Rust `==` agrees (both compare the nested set by identity; ResolvedFile variants compare the same fields; `Directory` path only in both). Caveat, weaker only: Java's `EMPTY` singleton (:52) equals itself; Rust `empty()` (`:458`) builds a fresh `NestedSet` Arc each call (`nested_set.rs:195`), so two empty results compare unequal. Safe (less pruning). In practice Java also prunes only on `EMPTY`/same-set. |

Counts: EQ-OK 1, EQ-WRITE 5 (2 substantive: ACTION_EXECUTION, ARTIFACT; 3 trivial singletons), IDENTITY 2, RUST-STRONGER 1, MIXED 0.

Other value-type notes:
- No Rust value in this crate holds `Arc<dyn Action>` in an equality-relevant position except `ActionTemplateExpansionValue` (identity anyway). `RunfilesArtifactValue` holds `Arc<dyn RunfilesTree>` (row 1). `BuildDriverValue` holds `Arc<dyn ConfiguredTargetView>` (identity anyway).
- `ActionLookupKey` impls at `skyframe_action_executor.rs:3772, 3952`, `action_execution_value.rs:257`, `action_execution_function.rs:1546` are inside `#[cfg(test)]` (test-start lines 3742, 226, 1498): excluded.

### B. Captured inputs (G1)

Registration: `lib.rs:57` `register_execution_functions(builder, executor, seam, unrunnable_test_reporter)`; production caller `crates/blaze_sky_top/src/execution_phase.rs:174` (also `crates/blaze_buildtool/src/bin/blaze-build-replay.rs:736`, a bin). The executor is built per command at `crates/blaze_buildtool/src/buildtool/execution_tool.rs:645`. Java builds `SkyframeActionExecutor` once per server (ctor `J:.../skyframe/SkyframeActionExecutor.java:274`) and sets per-build state in `prepareForExecution` (:331-380), `configure` (:1029-1035), `setClientEnv` (:387), clearing in `executionOver` (:482-496).

| Captured | Site | Field | Class | Java |
|---|---|---|---|---|
| `SkyframeActionExecutor` (all functions but ARTIFACT_NESTED_SET, TEST_COMPLETION) | `skyframe_action_executor.rs:319` | `exec_root` :320, `action_out_dir` :321, `external_directory` :322 | (a) | ctor/directories |
| | | `reporter` :323 | (c) per build field; used directly (see E/G5) | `reporter` set `SkyframeActionExecutor.java:339`, nulled :485 |
| | | `contexts` :324 | (c) | `executorEngine` :340, nulled :487 |
| | | `client_env` :325, read inside ACTION_EXECUTION at `action_execution_function.rs:508` and by the cache checker | **(b)**: Java does not read a captured env for the action; it requests `CLIENT_ENVIRONMENT_VARIABLE` per `action.getClientEnvironmentVariables()` (`J:.../skyframe/ActionExecutionFunction.java:220-240`, `ClientEnvironmentFunction.key`). PORT site `action_execution_function.rs:364`. `setClientEnv` :387 also exists (per command, for other users). |
| | | `keep_going` :326 | (d) | `options.getOptions(KeepGoingOption.class)` at :1026 (per build options) |
| | | `verbose_failures` :327, `output_permissions` :328, `explain` :331, `show_subcommands` :341, `output_filter` :339, `lost_inputs_policy` :343 | (c) per-build options | `options` set :352, nulled :486; `rewindingEnabled` :355 |
| | | `action_key_context` :329 (new per executor at :386) | (a) | ctor arg `actionKeyContext` (server lifetime), `J:.../actions/ActionKeyContext.java:22`, `clear()` :50 (callers not checked) |
| | | `action_cache_checker` :330, `action_execution_salt` :331, `containing_package_root_lookup` :333, `output_checker` :335 | (c) | `actionCacheChecker` :350 / nulled :494; salt via `configure` :1029 |
| | | `output_directory_helper` :337 | (c) | `outputDirectoryHelper` set in `prepareForExecution`, nulled :495 |
| | | `action_cache_purged: RwLock<bool>` :355 | no Java counterpart (PORT) | must reset per build |
| | | `per_build_file_cache: OnceLock<Arc<SingleBuildFileCache>>` :360, set by `configure_file_cache` :655 which **panics on a second call** (:656-658) | (c), blocks reuse | `perBuildFileCache` replaced per build `SkyframeActionExecutor.java:1035` |
| | | `build_action_map: Mutex<HashMap>` :362 | (c) | reset `prepareForExecution` :346, nulled :490 |
| | | `had_execution_error: AtomicBool` :363 | (c) | reset :349 |
| | | `action_uniquifier: AtomicU64` :365 | (c) | `ActionLogBufferPathGenerator` set per build `setActionLogBufferPathGenerator` :382 |
| `Arc<dyn AnalysisSeam>` (`SkyframeAnalysisSeam`, `crates/blaze_sky_top/src/execution_seam.rs:109`) | `lib.rs:60` | `services: Arc<AnalysisServices>` | other group (G1 table row `AnalysisServices`) | seam requests deps through `env` |
| `RecursiveFilesystemTraversalFunction` | `recursive_filesystem_traversal_function.rs:684` | `external_directory` :686 | (a) | `SkyframeExecutor.skyFunctions` ctor arg |
| `BuildDriverFunction` | `build_driver_function.rs:68` | `seam`; `key_to_posted_events: Mutex<HashMap<..>>` :72 | (c): needs a reset hook | `keyToPostedEvents` `J:.../skyframe/BuildDriverFunction.java:118`, `resetStates` :495, called by `SkyframeExecutor.resetBuildDriverFunction` `J:.../skyframe/SkyframeExecutor.java:2616` |
| `UnrunnableTestReporter` | `test_completion_function.rs:28`, `lib.rs:61` | closure | production passes `None` (`execution_phase.rs:174-178`) | n/a |
| static `TrackSourceDirectoriesFlag::track_source_directories()` | `recursive_filesystem_traversal_function.rs:54` | constant default | (a) | JVM property, server constant |
| `BUILD_ID` | PORT `action_execution_function.rs:364` | dep not requested | (b) `PrecomputedValue.BUILD_ID` `J:.../skyframe/PrecomputedValue.java:88`, requested at `ActionExecutionFunction.java:216-217` when `Actions.dependsOnBuildId(action)` | missing precomputed (also in plan 2.5) |

### C. File-system reads bypassing FILE_STATE / DIRECTORY_LISTING_STATE (G2)

| Site | What | Java | Verdict |
|---|---|---|---|
| `artifact_function.rs:440` `path.stat_if_found(Symlinks::Follow)` in `create_source_value` (PORT :436-438) | stat of a source artifact | `env.getValueOrThrow(FileValue.key(path))` `J:.../skyframe/ArtifactFunction.java:279-284` | **G2 site** (the known one; plan said `:199`, now `:440`) |
| `artifact_function.rs:461` `FileArtifactValue::create_from_stat(&path, &stat)` | builds source metadata from the direct stat (may digest the file) | `FileArtifactValue.createForSourceArtifact(artifact, fileValue, xattrProvider)` `ArtifactFunction.java:305`; also `RegularFileStateValueWithMetadata` reuse :297 and `sourceArtifactsSeen.accumulate` | **G2** (same fix as :440: take metadata from the `FileValue`) |
| `skyframe_action_executor.rs:3448`, `:3455` (`skyframe_input_metadata_provider_get_input_metadata_checked`, PORT :3437-3446) | metadata of a **generated** input rebuilt by stat'ing the output file | reads the `ActionExecutionValue` from the graph: `J:.../skyframe/SkyframeInputMetadataProvider.java:186-192` | **needs decision** (prelim.): inside ACTION_EXECUTION (semi-hermetic), but with a kept graph a modified output is seen before `FilesystemValueChecker` invalidation, so metadata can differ from the graph value Java uses. |
| `crates/blaze_exec/src/exec/single_build_file_cache.rs:43` via `per_build()` `skyframe_action_executor.rs:3415, 3423` | source and non-artifact inputs | same: `perBuild.getInputMetadataChecked` `SkyframeInputMetadataProvider.java:169, 176` | allowed (Java does the same; cache is per build, see E/G6) |
| `skyframe_action_executor.rs:2336, 2405, 2615, 2643` (output metadata store: tree / file construction from file system) | stats of outputs | `ActionOutputMetadataStore` (`J:.../skyframe/ActionOutputMetadataStore.java:388, 585`) | allowed (ACTION_EXECUTION semi-hermetic; Java does the same) |
| action cache check (`check_action_cache` from `action_execution_function.rs:542`) | action cache + output stats | `SkyframeActionExecutor.checkActionCache` :782 | allowed (Java does the same) |
| `recursive_filesystem_traversal_function.rs:974, 1200, 1225` | `FileKey` / `DirectoryListingKey` deps | `RecursiveFilesystemTraversalFunction` same | fine (goes through the graph) |
| `recursive_filesystem_traversal_function.rs:1086` `create_for_normal_file_using_path` (reads file to digest) | content digest of a file whose `FileStateValue` has no digest | `withDigest` `J:.../skyframe/RecursiveFilesystemTraversalFunction.java:472` same | allowed (Java does the same; the FILE dep is there) |
| `action_execution_function.rs:1019-1023` (PORT) | not a read: requests the input's key as a dep where Java reads the done node without an edge | `getExistingEntryAtCurrentlyEvaluatingVersion` | note: extra edge only adds invalidation (safe); plan for I6 |

### D. Mutable state reachable from values (G4)

| Site | Reached from | Kind |
|---|---|---|
| `crates/blaze_actions/src/actions/artifact.rs:66` `generating_action_key: OnceLock` | `ActionExecutionValue` map keys, `ArtifactKey`, `MissingArtifactValue` source | set once at analysis, read by `Artifact::eq` (plan G4; Java `setGeneratingActionKey`) |
| `crates/blaze_collect/src/nestedset/nested_set.rs:105` `cached: Option<Mutex<Weak<Vec<E>>>>` | `RecursiveFilesystemTraversalValue.resolved_paths` | memo (fine) |
| `crates/blaze_actions/src/analysis/runfiles_support.rs:129` `cached_mapping: Mutex<Weak<..>>` | `ActionExecutionValue.rich_artifact_data` -> `RunfilesArtifactValue.runfiles_tree` | memo (fine) |
| `crates/blaze_actions/src/actions/action.rs:134` `inputs: RwLock<NestedSet<Artifact>>` (`set_inputs` :170, `update_inputs` :327); `crates/blaze_rules/src/cpp/cpp_compile_action.rs:767` (`discovered_modules` Mutex) | `ActionTemplateExpansionValue.actions` (and actions reached from analysis values) | real mutation after the value is returned (input discovery); Java does the same (`AbstractAction.updateInputs`); value is IDENTITY so no pruning effect, but needs the I6 decision about discovered inputs kept across builds (`skyframe_action_executor.rs:1039-1041` PORT on `freeDiscoveredInputsAfterExecution`) |
| `TopLevelSkyValue` `build_driver_function.rs:20` -> `ConfiguredTargetView` / `ConfiguredAspectView` | `BuildDriverValue` | not followed (analysis group) |
| none in `ActionExecutionValue`, `FileArtifactValue`, `TreeArtifactValue`, `MissingArtifactValue`, `ArtifactNestedSetValue`, completion values | | plain data |

### E. Output, caches, side effects

G5 (output not through `env.listener()`):
- No `eprintln!`/`println!`/stderr writes in non-test code of the crate (grep below).
- Captured `reporter` used inside ACTION_EXECUTION: `skyframe_action_executor.rs:858` (`print_error`), `:1321-1335`, `:1428`, `:1473`, `:1510` (explain handler), `:1582`, `:1929`, `:1939` (dumped stdout/stderr), `:3654` (`get_event_handler` of the action context), `:3666` (subcommands). Java does the same (`reporter` field `SkyframeActionExecutor.java:196`; `selectEventHandler` :706-707 returns `reporter`; :652, :1311-1312, :1572, :1598, :1669, :1748, :1769; Java's own comment :1090-1101 flags it). Not a Blaze deviation; on a later cached build these messages are not replayed in Java either. Classified (c) in B.
- All other sites use `env.listener()` (`action_execution_function.rs:543, 580`, `artifact_function.rs:230`, `build_driver_function.rs:98`, `completion_function.rs:157, 267, 863, 887, 895, 926`, `test_completion_function.rs:169, 174`, `skyframe_action_executor.rs:964, 1013, 1016`).

G6 (caches outside the graph consulted in functions):
| Site | What | Java | Verdict |
|---|---|---|---|
| `skyframe_action_executor.rs:362` `build_action_map` | shared-action results | reset per build `:346` | reset per build (I6) |
| `skyframe_action_executor.rs:360` `per_build_file_cache` -> `single_build_file_cache.rs:49` `path_to_metadata: Mutex<HashMap>` | source/non-artifact input metadata | per build (`configure` :1035) | must be replaced per build; today `OnceLock` + panic |
| `build_driver_function.rs:72` `key_to_posted_events` | posted-event dedup | `resetStates` `BuildDriverFunction.java:495` via `SkyframeExecutor.java:2616` | port the reset |
| `crates/blaze_exec/src/analysis/actions/mod.rs:268` static `ACTION_INPUT_SETS` (used by `to_action_input_set` :259) | `NestedSet<Artifact>` -> `NestedSet<ActionInputRef>` memo keyed by strong node | Rust-only (Java artifacts are ActionInputs) | correct (strong keys, no reuse), unbounded growth (plan 3.1 "reset") |
| `skyframe_action_executor.rs:329` `action_key_context` | action-key fingerprint cache | server lifetime, `ActionKeyContext.clear()` :50 | fine if executor becomes per server; check Java `clear` callers |
| `recursive_filesystem_traversal_function.rs:428` static `EMPTY: LazyLock<HasDigest>` | constant | | fine |

G7 (side effects inside functions):
| Site | Effect | Java | Note |
|---|---|---|---|
| `artifact_function.rs:224` `create_directory_and_parents` for template tree outputs | mkdir | `ArtifactFunction.mkdirForTreeArtifact` `ArtifactFunction.java:191` | repeatable |
| `skyframe_action_executor.rs:988-996` `executable.prepare` (deletes outputs) | delete | `ActionRunner.run` eager prep `:1128` | repeatable; Java same |
| `skyframe_action_executor.rs:2020, 2025` stdout/stderr dirs | mkdir | `setupActionFsFileOutErr` `:1495` | repeatable |
| `skyframe_action_executor.rs:2417` chmod outputs | chmod | `ActionOutputMetadataStore.setPathPermissions` | repeatable |
| action spawns, action cache writes (`update_action_cache`, `action_execution_function.rs:632`) | processes, cache journal | same | Java rules |
| `skyframe_action_executor.rs:2697` PORT `notifyDependenceOnFileTime` (plan said `:2615`) | TSGM not notified | `J:.../actions/FileStateValue.java:163`, `ActionOutputMetadataStore.java:585` | needed with a kept graph (I3/I6) |
| `skyframe_action_executor.rs:475` `action_cache_purged` write | purge of action cache (Rust-only) | none | flag must reset per build |

`PORT:[^\n]*incremental` in the crate (`grep -rniE "PORT:[^\n]*incremental"`): `action_execution_function.rs:343` (remote analysis caching), `:364` (BUILD_ID and CLIENT_ENVIRONMENT_VARIABLE deps), `skyframe_action_executor.rs:2697` (TSGM). Loose `incremental` mentions also at `action_execution_function.rs:1023`, `artifact_function.rs:437`, `skyframe_action_executor.rs:1039-1040`. Strict `PORT: skipped, incremental` = 3 lines (matches plan 2.6).

### Greps run
- `grep -rn "impl.*Key for\|impl Key" crates/blaze_sky_exec/src`
- `grep -n "typed_function"` across crates for the 9 keys; `grep -rn "register_execution_functions\|SkyframeActionExecutor::new"`
- Java: `grep -n "class \|equals\|hashCode\|NotComparable\|INSTANCE\|@AutoValue"` on ActionExecutionValue, ArtifactNestedSetValue, Target/Aspect/TestCompletionValue, BuildDriverValue, ExclusiveTestBuildDriverValue, ActionTemplateExpansionValue, BasicActionLookupValue, RecursiveFilesystemTraversalValue, FileArtifactValue, TreeArtifactValue, RunfilesArtifactValue, NestedSet; `SkyFunctions.java` for the 9 names; `Artifact.java:159`
- `grep -rn "INSTANCE\|TEST_COMPLETION_MARKER"` (singleton return sites)
- `grep -nE "std::fs|fs::|stat\(|stat_if_found|read_dir|readlink|exists\(|is_file|is_directory|metadata\(|read_to_string|open\(|create_|delete|chmod"` over crate src
- `grep -nE "static |thread_local!|eprintln!|println!|eprint!|stderr|Box::leak"` over crate src
- awk over non-test regions for `reporter|listener()|event_handler`
- `grep -n "OnceLock|Mutex|RwLock|Atomic|RefCell"` in value-reachable files
- `grep -rniE "PORT:[^\n]*incremental"` and `"PORT: skipped"` in crate
- Java `SkyframeActionExecutor.java` for `reporter`, `prepareForExecution`, `executionOver`, `configure`, `perBuildFileCache`; `BuildDriverFunction.java` `keyToPostedEvents`

### Not checked
- Whether Rust C++ module discovery (`discovered_modules`) ever produces values that Java's `ActionExecutionValue.getDiscoveredModules()` would carry (row 1 blocker 3).
- `ConfiguredTargetView` / `ConfiguredAspectView` interior state (analysis group).
- Callers of Java `ActionKeyContext.clear()`.
- `FileArtifactValue::create_from_stat` internals (whether it digests eagerly) vs `createForSourceArtifact`.
- `ActionCacheChecker` internals (in `blaze_exec`), `SingleBuildFileCache` read paths beyond the struct.
- `ActionTemplateExpansionFunction` body in `blaze_sky_top` beyond a grep for fs/static/reporter (none found).
- Key `Eq`/`Hash` of `ActionExecutionKey`/`ArtifactKey` vs Java (derived over `ActionLookupData`/`Artifact`; not audited).

### F. Group 5 worker: files, order, hand-back

Files:
- `crates/blaze_sky_exec/src/{keys.rs, action_execution_value.rs, artifact_function.rs, completion_function.rs, test_completion_function.rs, build_driver_function.rs, action_template_expansion_value.rs, recursive_filesystem_traversal_function.rs, action_execution_function.rs, skyframe_action_executor.rs}`
- Cross-crate, assigned here: `crates/blaze_exec/src/actions/input_metadata_provider.rs`
  (`TreeArtifactValue: PartialEq`; a hand-written `RunfilesArtifactValue` equality that ignores
  `runfiles_tree`, as Java `RunfilesArtifactValue.java:169-175`).

Order:
1. `HERMETICITY` on the 9 keys:
   - `ACTION_EXECUTION` is SemiHermetic;
   - `BUILD_DRIVER` is NonHermetic;
   - `ARTIFACT` is Hermetic (`J:.../actions/Artifact.java:159`, with its comment).
2. The completion singletons (3): derive `PartialEq` on the unit structs, or return `true`.
3. `ActionExecutionValue`: needs `TreeArtifactValue` and `RunfilesArtifactValue` equality. Settle the
   discovered-modules question (Java `shallowEquals`) before opting in.
4. `ArtifactValue`: a hand-written `values_equal` (`File` by `FileArtifactValue ==`, `Tree` by the new
   equality, `Missing` never equal).
5. `RecursiveFilesystemTraversalValue`: opt in.
6. Do not opt in `ArtifactNestedSetValue` (NotComparable). `BuildDriverValue` and
   `ActionTemplateExpansionValue` stay IDENTITY.
7. G2: the source artifact stat (`artifact_function.rs:440, 461`) becomes a `FileKey` dependency plus
   `FileArtifactValue.createForSourceArtifact` (`ArtifactFunction.java:279-305`). Record a decision on
   `skyframe_action_executor.rs:3448, 3455`: Java reads the `ActionExecutionValue` from the graph,
   `SkyframeInputMetadataProvider.java:186-192`.
8. G1:
   - ACTION_EXECUTION must request `CLIENT_ENVIRONMENT_VARIABLE` per `getClientEnvironmentVariables()`
     (`ActionExecutionFunction.java:220-240`) instead of reading `client_env` (`:508`). This needs X2,
     because `blaze_sky_exec` cannot name the bzlmod key today;
   - the `BUILD_ID` dependency (`:364`) needs X1 plus the precomputed value.
9. Per-build state (with I6):
   - `per_build_file_cache` must stop panicking on a second `configure_file_cache`;
   - `build_action_map`, `had_execution_error`, `action_uniquifier` and `action_cache_purged` must be
     reset per build;
   - `BuildDriverFunction.key_to_posted_events` needs a reset hook (`BuildDriverFunction.java:495`,
     `SkyframeExecutor.java:2616`).

Hand back: one line per key. Per `SkyframeActionExecutor` field: its class and what was done. Tests:
- `ActionExecutionValue` equality with the same outputs and with one changed digest;
- the source artifact edit test (it needs I1);
- the client environment variable change of an action that reads it.

## Group 6: blaze_sky_top (6 keys) and the blaze_skyframe key

Helper draft: **Group 6: `blaze_sky_top` (6 keys), plus the `blaze_skyframe` key**

Tree: HEAD `4f92912` plus uncommitted work of others (line numbers as of 2026-11-19). Read-only survey; nothing built.
`J:` = `~/.cache/blaze/src/bazel-9.3.2/src/main/java/`. `SkyFunctions.java` = `J:com/google/devtools/build/lib/skyframe/SkyFunctions.java`.

### A. Keys

| # | Rust key | FUNCTION | Rust value type | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | `crates/blaze_sky_top/src/action_lookup_conflict_finding_function.rs:84` `ActionLookupConflictFindingKey` | `ACTION_LOOKUP_CONFLICT_FINDING` = `"ACTION_LOOKUP_CONFLICT_DETECTION"` (`:28-29`) | `ActionLookupConflictFindingValue` `:47` (unit struct) | none (`Clone, Debug, Default`) | `J:.../skyframe/ActionLookupConflictFindingValue.java:50` `Key extends AbstractSkyKey<ActionLookupKey>` | `J:.../skyframe/ActionLookupConflictFindingValue.java:32`: no `equals`, but the only instance is the singleton `INSTANCE` (:34), so it is always equal to itself | Hermetic, `SkyFunctions.java:97-98` | **EQ-WRITE** (prelim.). Singleton: `values_equal` = `true`. Nothing blocks it. The Rust value is a fresh `Arc` each time, so derive `PartialEq` or write `true`. |
| 2 | `crates/blaze_sky_top/src/action_lookup_conflict_finding_function.rs:123` `TopLevelActionLookupConflictFindingKey` | `TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING` = `"TOP_LEVEL_ACTION_LOOKUP_CONFLICT_DETECTION"` (`:32-33`) | `ActionLookupConflictFindingValue` `:47` | none | `J:.../skyframe/TopLevelActionLookupConflictFindingFunction.java:72` `@AutoValue Key(actionLookupKey, topLevelArtifactContext)` | same singleton (`TopLevelActionLookupConflictFindingFunction.java:57` returns `ActionLookupConflictFindingValue.INSTANCE`) | Hermetic, `SkyFunctions.java:99-100` | **EQ-WRITE** (prelim.). Same as #1. Key note: Rust keeps only `outputGroups` of `TopLevelArtifactContext` (`:92-100`), so the key is coarser than Java's AutoValue. That is harmless while the other context fields are constant, but should be recorded. |
| 3 | `crates/blaze_sky_top/src/target_pattern_phase.rs:279` `TargetPatternErrorKey` | `TARGET_PATTERN_ERROR` (`:53`) | `TargetPatternErrorValue` `:277` (never produced: the function always throws, `:285-290`) | none | `J:.../skyframe/TargetPatternErrorFunction.java:43` (`equals` :74: message + detailedExitCode) | no value class: `TargetPatternErrorFunction.compute` always throws (`TargetPatternErrorFunction.java:91`) | Hermetic, `SkyFunctions.java:70-71` | **IDENTITY** (prelim.). There is never a value, so leave the default. Key note: the Rust `Eq` is derived over the whole `TargetParsingException` (`crates/blaze_cmdline/src/target_parsing_exception.rs:46-54`: message, code, detailed_exit_code), while `Hash` uses message + code (`:245-250`). It is stricter than Java's message + `DetailedExitCode`, which only splits keys Java would merge (already noted in the plan). |
| 4 | `crates/blaze_sky_top/src/target_pattern_phase.rs:333` `TestExpansionKey` | `TESTS_IN_SUITE` (`:51`) | `TestExpansionValue` `:323` | none (`Clone, Debug`) | `J:.../skyframe/TestExpansionValue.java:58` `TestExpansionKey` (`functionName()` = `SkyFunctions.TESTS_IN_SUITE`, :69) | `J:.../skyframe/TestExpansionValue.java:34`: no `equals`, so identity | Hermetic, `SkyFunctions.java:88` | **IDENTITY** (prelim.). |
| 5 | `crates/blaze_sky_top/src/target_pattern_phase.rs:565` `TestsForTargetPatternKey` | `TEST_SUITE_EXPANSION` (`:49`) | `TestsForTargetPatternValue` `:555` | none (`Clone, Debug`) | `J:.../skyframe/TestsForTargetPatternValue.java:59` `TestsForTargetPatternKey` (`functionName()` = `SkyFunctions.TEST_SUITE_EXPANSION`, :68; `equals` :86 over an `ImmutableSortedSet<Label>`) | `J:.../skyframe/TestsForTargetPatternValue.java:36`: no `equals`, so identity | Hermetic, `SkyFunctions.java:86-87` | **IDENTITY** (prelim.). Key note: Rust `targets: IndexSet<Label>`; derived `Eq` is order-insensitive and `Hash` is a manual order-insensitive sum (`:518-528`), which agrees with Java's sorted set. |
| 6 | `crates/blaze_sky_top/src/target_pattern_phase.rs:855` `TargetPatternPhaseKey` | `TARGET_PATTERN_PHASE` (`:47`) | `TargetPatternPhaseValue` `:809` | derived `:808`, over `target_labels`, `tests_to_run_labels`, **`non_expanded_labels`**, `has_error`, `has_post_expansion_error` (IndexSet eq is order-insensitive, like `ImmutableSet.equals`) | `J:.../skyframe/TargetPatternPhaseValue.java:181` (`equals` :286) | `J:.../skyframe/TargetPatternPhaseValue.java:44`, `equals` at :118 compares `targetLabels`, `testsToRunLabels`, `hasError`, `hasPostExpansionError`, and **not `nonExpandedLabels`** | **NonHermetic**, `SkyFunctions.java:90-91` | **EQ-WRITE** (prelim.). The derived `==` also compares `non_expanded_labels`, which Java's `equals` ignores. With the derived `==`, a change that only touches `non_expanded_labels` would propagate in Rust but be pruned in Java. Write a manual `values_equal` without that field. All field types already have `PartialEq`. |

Verdict counts (group 6): EQ-WRITE 3, IDENTITY 3, EQ-OK 0.

**FUNCTION name check (not swapped).** Rust `TestExpansionKey` maps to `TESTS_IN_SUITE`, and Java `TestExpansionValue.TestExpansionKey.functionName()` returns `SkyFunctions.TESTS_IN_SUITE` (`TestExpansionValue.java:69`). Rust `TestsForTargetPatternKey` maps to `TEST_SUITE_EXPANSION`, and Java `TestsForTargetPatternKey.functionName()` returns `SkyFunctions.TEST_SUITE_EXPANSION` (`TestsForTargetPatternValue.java:68`). They match Java. The pairing looks swapped by name, but it is Java's own. The Rust constant `ACTION_LOOKUP_CONFLICT_FINDING` carries the string `"ACTION_LOOKUP_CONFLICT_DETECTION"`, and `TOP_LEVEL_...` carries `"TOP_LEVEL_ACTION_LOOKUP_CONFLICT_DETECTION"`. Both are Java's strings (`SkyFunctions.java:97-100`: the Java constant name is `..._FINDING`, the string is `..._DETECTION`). Correct.

### A'. `blaze_skyframe` key

| # | Rust key | FUNCTION | Rust value type | Rust PartialEq today | Java key class | Java value class and equality | Java hermeticity | Verdict |
|---|---|---|---|---|---|---|---|---|
| 7 | `crates/blaze_skyframe/src/evaluator.rs:46` `ErrorTransienceKey` | `ERROR_TRANSIENCE` | `ErrorTransienceValue` `evaluator.rs:56` (unit struct) | none (`Debug`) | `J:com/google/devtools/build/skyframe/ErrorTransienceValue.java:28` anonymous `KEY` (`valueIsShareable() = false`) | `J:com/google/devtools/build/skyframe/ErrorTransienceValue.java:22`: `equals` at :59 **always returns `false`** ("Is not equal to anything, including itself, in order to force re-evaluation", :20); `hashCode` throws | **NonHermetic**, `ErrorTransienceValue.java:25` | **IDENTITY** (prelim.). Must NOT opt in, and must not be given a `PartialEq`. The default `false` is exactly Java's semantics. **Warning for I1:** if the evaluator adds an `Arc::ptr_eq` short-cut before calling `values_equal`, it would break this node: the injected value is a fresh `Arc` each time (`evaluator.rs:473`), but any pointer short-cut contradicts Java's "not even itself". |

### B. Inputs that are not dependencies (G1)

#### Group 6 functions themselves

| Site | Captured / read | Class | Java |
|---|---|---|---|
| `crates/blaze_sky_top/src/action_lookup_conflict_finding_function.rs:201-203` `ActionLookupConflictFindingFunction { action_conflicts: ActionConflictsCell }`, read at `:213`; written by `SkyframeExecutor::filter_action_conflicts_for_configured_targets_and_aspects` `crates/blaze_sky_top/src/skyframe_executor.rs:697`; cell created at `skyframe_executor.rs:344-345`, registered at `:437-440` | the current action conflicts (`Arc<RwLock<Arc<ActionConflicts>>>`) | (b) `ArtifactConflictFinder.ACTION_CONFLICTS` `J:.../skyframe/ArtifactConflictFinder.java:43-44` (`new Precomputed<>("action_conflicts")`), read `ACTION_CONFLICTS.get(env)` `J:.../skyframe/ActionLookupConflictFindingFunction.java:46`, set `J:.../skyframe/SkyframeExecutor.java:2548` | Becomes a precomputed dep. Also needs Java's node deletion (see E). Even with the dep, Java deletes these nodes after each use. |
| `TopLevelActionLookupConflictFindingFunction` `action_lookup_conflict_finding_function.rs:280` | nothing | fine | |
| `register_target_pattern_phase_functions` `crates/blaze_sky_top/src/target_pattern_phase.rs:1360-1366` | plain `fn`s, no captured state, no `env.client` | fine | Java functions have no fields used here (no `PrecomputedValue` reads in `TargetPatternPhaseFunction`, `TestsForTargetPatternFunction`, `TestExpansionFunction`) |
| events in target-pattern functions | all through `env.listener()` (`target_pattern_phase.rs:386, 471, 499, 944, 1002, 1076, 1123, 1132, 1140, 1162, 1167`) | fine (G5) | Java `env.getListener()` |

#### Every registration in `crates/blaze_sky_top/src/skyframe_executor.rs` (`create_with_builtin_modules`, :217-522)

| Line | Registration | Owner crate | Captured state | Class (prelim.) |
|---|---|---|---|---|
| :255-277 | `blaze_repo_rules::setup::prepare_repository_functions_with(FetchSetup{..., reporter: inputs.reporter.clone()})` | `blaze_repo_rules` | `reporter` goes into `DownloadManager::new(.., reporter.clone())` (`crates/blaze_repo_rules/src/setup.rs:431-436`), held by `RepoServices.download_manager` (`crates/blaze_repo_rules/src/services.rs:27`); also client/repo env, timeouts, process wrapper, repository cache, repo-contents-cache lock (`setup.rs:424-430`) | reporter: (c) Java `BazelRepositoryModule.beforeCommand` builds a new `DownloadManager(…, env.getReporter())` per command and calls `setDownloadManager` on the functions (`J:.../bazel/BazelRepositoryModule.java:276-287`), so it is a per-command setter. `repo_env`/`nonstrict_repo_env`: (c) Java `repoEnvSupplier.set(env.getRepoEnv())` (:289-290). Owner group decides. |
| :317-320 | `register_bzlmod_functions(EvaluatorBuilder::new(), Arc::new(bzlmod_config))` | `blaze_bzlmod` | `BzlmodConfig` (`crates/blaze_bzlmod/src/skyframe/config.rs:105-145`): registries, module_mirrors, ignore_dev_deps, module_overrides, injected_repositories, check_direct_dependencies, bazel_compatibility_mode, lockfile_mode, allowed_yanked_versions, vendor_directory, last_registry_invalidation, client_env, starlark_semantics, registry fetcher/factory, builtin modules | mostly (b): Java `Precomputed` `registries` `J:.../bazel/bzlmod/ModuleFileFunction.java:107`, `ignore_dev_dependency` :109, `module_overrides` :112, `repository_injections` :114, `last_registry_invalidation` `RegistryFunction.java:43`, `module_mirrors` :46, `lockfile_mode` `BazelLockFileFunction.java:51`, `check_direct_dependency`/`bazel_compatibility_mode` `BazelModuleResolutionFunction.java:65,67`, `ALLOWED_YANKED_VERSIONS` `YankedVersionsUtil.java:28`, `VENDOR_DIRECTORY` `J:.../rules/repository/RepositoryDirectoryValue.java:48`, `STARLARK_SEMANTICS`; `client_env` is a CLIENT_ENVIRONMENT_VARIABLE dep in Java. Owner group decides. |
| :321 | `register_loading_functions(builder, &loading_config)` | `blaze_sky_loading` | `LoadingFunctionsConfig` (already the first row of plan §2.5 G1) | owner group |
| :323-329 | `inject_loading_precomputed_with_package_options(..)` | `blaze_sky_loading` | injects precomputed values (graph state) | fine (already precomputed) |
| :330-333 | `register_repo_rules_functions(builder, repository_functions.services)` | `blaze_repo_rules` | `Arc<RepoServices>` (see :255 row) | owner group |
| :334-335 | `inject_repo_precomputed(builder, &repo_precomputed)` | `blaze_repo_rules` | precomputed (`FETCH_DISABLED`, `FORCE_FETCH`, …, `RepositoryDirectoryValue.java:42-48`) | fine |
| :336 | `register_target_pattern_phase_functions` | `blaze_sky_top` | none | fine |
| :339 | `crate::project::register` = re-export of `blaze_sky_loading::project_function::register` (`crates/blaze_sky_top/src/project.rs:27-30`, `crates/blaze_sky_loading/src/project_function.rs:962-966`) | `blaze_sky_loading` | none (plain fns) | fine |
| :393-407 | `register_configuration_functions(builder).client(ConfigurationEnvironment)` | `blaze_build_config` | see `g3.md` §B | (a)/(b) |
| :410-417 | `blaze_sky_analysis::parsed_flags_function::register(builder, options_classes)` | `blaze_sky_analysis` | `Arc<Vec<FragmentOptionsClass>>` | (a) |
| :420 | `build_options_scope_function::register` | `blaze_sky_analysis` | none | fine |
| :421-424, :425-426, :429, :430-433 | `register_toolchain_functions`, `configured_target_function::register`, `aspect_function::register`, `toplevel_starlark_aspect_function::register`, each with `services.clone()` | `blaze_sky_analysis` | `Arc<AnalysisServices>` (`crates/blaze_analysis/src/analysis_services.rs`): `starlark_semantics` (b) STARLARK_SEMANTICS; `keep_going` (d) per evaluation (built from `inputs.keep_going` at :375); `main_repo_mapping: RwLock` written per command at `skyframe_executor.rs:747` (Java: functions request `RepositoryMappingValue`, so it should become a dep); `workspace_status*` (below); `base_target_prerequisites_supplier` (below); `artifact_factory`, `action_key_context`, `directories`, `rule_class_provider`, `bazel_starlark_environment`: (a) | owner `blaze_analysis`/`blaze_sky_analysis` |
| :434 | `load_aspects_function::register` | `blaze_sky_analysis` | none | fine |
| :437-440 | `action_lookup_conflict_finding_function::register(builder, action_conflicts.clone())` | `blaze_sky_top` | `ActionConflictsCell` | (b), see above |
| :441-446 | `typed_function::<WorkspaceStatusKey>(move \|_, _\| Ok(Some(status_services.compute_workspace_status_value())))`, closure in `blaze_sky_top` capturing `Arc<AnalysisServices>`, body in `crates/blaze_analysis/src/analysis_services.rs:109` | key/value `blaze_analysis`; closure `blaze_sky_top` | `workspace_status: OnceLock<WorkspaceStatusArtifacts>` (`analysis_services.rs:55`, filled by `get_or_init` at :136-137, created at `skyframe_executor.rs:376`); `workspace_status_user_name` / `host_name` (`skyframe_executor.rs:379-380`) | OnceLock: (a) a memo of artifacts that depend only on directories and the artifact factory. Java recreates the action per compute via `makeWorkspaceStatusAction` (`J:.../skyframe/SkyframeExecutor.java:1487-1511`, registered `:931`); the derived artifacts are deduped by `ArtifactFactory`, so equivalent. User/host name: (a) server constants (Java `StandardSystemProperty.USER_NAME`, `NetUtil.getCachedShortHostName()`). BUILD_INFO is Hermetic (`SkyFunctions.java:128`). |
| :447-465 | `crate::execution_phase::prepare_execution(builder, setup, &services, inputs.reporter.clone())` (`crates/blaze_sky_top/src/execution_phase.rs:88-206`) | `blaze_sky_top` (setup) | registers `blaze_sky_exec::register_execution_functions(builder, executor, seam, None)` (`execution_phase.rs:174-179`, owner `blaze_sky_exec`) capturing `Arc<SkyframeActionExecutor>` (built with the command's `reporter`, `execution_phase.rs:136-158`) and `SkyframeAnalysisSeam(services)`. It also registers `typed_function::<ActionTemplateExpansionKey>` (`execution_phase.rs:182-191`), a `blaze_sky_top` function (`action_template_expansion_function.rs`) capturing executor + seam. Action contexts capture `setup.embed_label`, `workspace_status_command`, and `env.client_env` (`execution_phase.rs:113-121`). | executor/reporter: (c) Java `SkyframeActionExecutor.prepareForExecution` sets `this.reporter` per build (`J:.../skyframe/SkyframeActionExecutor.java:331-339`) and clears it at :485. The executor object is server-lifetime in Java. Action contexts: (c) per command (`registerActionContexts`). Owner of the exec functions: `blaze_sky_exec`. |
| :470-485 | after `build()`: `services.base_target_prerequisites_supplier.set(..)` with a weak evaluator handle | `blaze_sky_top` → `blaze_analysis` | `OnceLock` set once | (a) Java `BaseTargetPrerequisitesSupplierImpl` (`SkyframeExecutor.java:4598`) |
| :167, :668-678 | `top_level_artifact_context_for_conflict_pruning: Mutex<Option<Vec<String>>>` | `blaze_sky_top` | executor field read by `skyframe_build_view::configure_targets`, not by a SkyFunction | (d) per request; outside the graph, fine |

### C. File-system reads bypassing FILE_STATE (G2)

None in `blaze_sky_top` SkyFunctions. The only `std::fs`/`exists`/`is_dir` hits are in `crates/blaze_sky_top/src/testing/` (`mock_tools_config.rs:41-112`, `scratch.rs:43-184`, `build_view_test_case.rs:276-278, 543`). That is test support (`pub mod testing`, `crates/blaze_sky_top/src/lib.rs:42`), not called from SkyFunctions. The `ExecutionTool` exec-root creation (`execution_phase.rs:96-100`) runs at setup, not inside a function.

### D. Mutable state in values (G4)

| Site | Reached from | Verdict |
|---|---|---|
| `ActionLookupConflictFindingValue` (`action_lookup_conflict_finding_function.rs:47`) | unit struct | none |
| `TestExpansionValue`/`TestsForTargetPatternValue` (`target_pattern_phase.rs:323, 555`): `ResolvedTargets<Label>` | no interior mutability found in `crates/blaze_cmdline/src/resolved_targets.rs` | fine |
| `TargetPatternPhaseValue` (`:809`): `IndexSet<Label>` + bools | | fine |
| `ErrorTransienceValue` (`crates/blaze_skyframe/src/evaluator.rs:56`) | unit | fine |
| `WorkspaceStatusValue` (closure at `skyframe_executor.rs:443-446`) holds artifacts from the `OnceLock` (`analysis_services.rs:55`). Artifacts carry `generating_action_key: OnceLock` (`crates/blaze_actions/src/actions/artifact.rs:66`), already flagged in plan §2.5 G4 | | owner `blaze_actions` decision |

### E. Events, caches, side effects (G5/G6/G7)

| Site | Kind | Java | Verdict |
|---|---|---|---|
| `crates/blaze_sky_top/src/execution_seam.rs:857-865` `static TOP_LEVEL_ASPECT_DRIVERS { analyzed: AtomicUsize, created: AtomicBool }` (the "progress counter" of plan §3.1 at :850 is now this guard; no progress counter remains in `execution_seam.rs`). It is written by `build_view.rs:355` (`note_top_level_aspects_analyzed_for_execution`, `store`, not add) and `execution_seam.rs:907-909`, and read **inside a SkyFunction helper**, `SkyframeAnalysisSeam::get_configured_target` → `check_top_level_aspect_drivers_created()` (`execution_seam.rs:218`) | process-wide static consulted inside execution functions (G6) | none (Blaze-only guard, PORT at `:849-855`) | Needs a decision: move it to per-command state, e.g. the `ExecutionState` or the seam. It is reset per command only on the `build_view.rs:355` path; a command that skips that path sees the previous command's values. Low risk (it only gates an `unported!`). |
| `crates/blaze_sky_top/src/skyframe_executor.rs:684-687` PORT: skipped, incremental: conflict-finding nodes are not deleted | missing graph deletion | `J:.../skyframe/SkyframeExecutor.java:2560-2561` `memoizingEvaluator.delete(functionIs(TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING))` after each evaluation, and `resetActionConflictsStoredInSkyframe` `:2611-2613` deletes `ACTION_LOOKUP_CONFLICT_FINDING` (called `J:.../skyframe/SkyframeBuildView.java:577, 1020`) | **Must port with a kept graph**: with the cell (no dep edge), stale conflict nodes would be reused by the next build. Even after ACTION_CONFLICTS becomes a precomputed dep, Java deletes them. Needs `Evaluator::delete(predicate)` (I1/I5). |
| `crates/blaze_sky_top/src/skyframe_build_view.rs:402-403` PORT: skipped, incremental: `resetActionConflictsStoredInSkyframe` on the nokeep_going path | same | `SkyframeBuildView.java:1020` | same as above |
| `crates/blaze_sky_top/src/skyframe_build_view.rs:195-196` PORT: skipped, incremental: `enableAnalysis` | analysis-node reuse guard | `SkyframeBuildView.enableAnalysis` | I5 (per plan §2.6) |
| `crates/blaze_sky_top/src/skyframe_build_view.rs:359-361` PORT: skipped, incremental: "ConfiguredTarget has been cleared" | analysis-object discarding | `SkyframeBuildView` | I5 |
| `crates/blaze_sky_top/src/build_view.rs:297` PORT: `ArtifactFactory.noteAnalysisStarting` only resets incremental state | per-build reset of the artifact factory | `ArtifactFactory.noteAnalysisStarting` | I5. Note: with a server-lifetime `ArtifactFactory` (it lives in `AnalysisServices`), this reset becomes necessary. |
| `crates/blaze_sky_top/src/cycles_reporter.rs:58` `cycle_deduper: Mutex<CycleDeduper>` | executor field, used outside SkyFunctions | Java `CyclesReporter` per executor | fine for G6 (not consulted by functions). Whether the deduper must reset per command was not checked. |
| `eprintln!`/`println!`/stderr in `crates/blaze_sky_top/src` | none outside `testing/` | | fine |
| G7 in group-6 functions | none (conflict finding and target patterns are pure graph reads) | | fine |

### Greps run
- `grep -n 'impl Key for\|type Value\|const FUNCTION\|SkyFunctionName(\|derive\|impl PartialEq' crates/blaze_sky_top/src/{target_pattern_phase,action_lookup_conflict_finding_function}.rs`
- `grep -n 'class \|record \|SkyFunctions\.\|equals\|hashCode\|NotComparable' J:` TestExpansionValue, TestsForTargetPatternValue, TargetPatternPhaseValue, TargetPatternErrorFunction, ActionLookupConflictFindingValue, TopLevelActionLookupConflictFindingFunction; `J:com/google/devtools/build/skyframe/ErrorTransienceValue.java` read in full (:20-80)
- `grep -n -E 'CONFLICT|TARGET_PATTERN|TESTS_IN_SUITE|TEST_SUITE|BUILD_INFO' SkyFunctions.java`
- `grep -n 'PORT:[^\n]*incremental' crates/blaze_sky_top/src/*.rs` (4 hits: `skyframe_build_view.rs:195, 359, 402`, `skyframe_executor.rs:684`); `build_view.rs:297` found by `grep incremental`
- `grep -n 'static \|OnceLock\|Atomic\|Mutex\|RwLock\|thread_local\|eprintln\|println\|std::fs\|std::io::stderr' crates/blaze_sky_top/src/*.rs`
- `grep -rn 'std::fs\|read_to_string\|\.exists()\|stat_if_found\|read_dir\|is_dir()\|metadata\|get_input_stream' crates/blaze_sky_top/src`
- `grep -n 'register\|typed_function\|\.function(\|\.write()\|\.set(\|OnceLock' crates/blaze_sky_top/src/skyframe_executor.rs`
- `grep -n 'ExtendedEventHandler\|eventHandler' J:.../bazel/repository/downloader/DownloadManager.java`; `J:.../bazel/BazelRepositoryModule.java:270-300`
- `grep -rn 'new Precomputed<' J:` (all Java precomputed constants)
- `grep -n 'resetActionConflictsStoredInSkyframe\|memoizingEvaluator.delete\|ACTION_CONFLICTS' J:.../skyframe/{SkyframeExecutor,SkyframeBuildView,ArtifactConflictFinder,ActionLookupConflictFindingFunction}.java`
- `grep -n 'prepareForExecution\|this.reporter' J:.../skyframe/SkyframeActionExecutor.java`
- `grep -rn 'values_equal\|HERMETICITY' crates/blaze_skyframe/src`: nothing yet (I1 not in the tree)

### Not checked
- The bodies of `target_pattern_phase_function` / `test_expansion_function` beyond events and deps (assumed pure graph reads, apart from the `env.listener()` events listed).
- `skyframe_executor.rs:760-1672` beyond grepping for registrations and cell writes (no further registrations found).
- Field-by-field classification of `BzlmodConfig`, `RepoServices`, `LoadingFunctionsConfig`, `AnalysisServices`, `SkyframeActionExecutor`: the owning crates' groups.
- Whether `CyclesReporter`'s deduper must be reset per command.
- How I1 attaches `values_equal` to functions registered with untyped `.function(..)` (both conflict-finding functions, `action_lookup_conflict_finding_function.rs:376-383`).

### F. Group 6 worker: files, order, hand-back

Files:
- `crates/blaze_sky_top/src/{action_lookup_conflict_finding_function.rs, target_pattern_phase.rs, skyframe_executor.rs, execution_seam.rs, execution_phase.rs, build_view.rs, skyframe_build_view.rs}`
- `crates/blaze_sky_top/src/action_template_expansion_function.rs`: the function of a group 5 key. Only
  its captured state is audited here.
- `ErrorTransienceKey`: owned by I1 (see the header).

Order:
1. `HERMETICITY` on the 6 keys (`TARGET_PATTERN_PHASE` is NonHermetic).
2. Write `values_equal`:
   - conflict-finding singletons: `true`;
   - `TargetPatternPhaseValue`: Java's four fields, **without `non_expanded_labels`**.
3. The three IDENTITY keys stay at the default.
4. G1: the action-conflicts cell becomes the `action_conflicts` precomputed value
   (`J:.../skyframe/ArtifactConflictFinder.java:43-44`), after X1. The conflict-finding nodes are
   deleted after use (`SkyframeExecutor.java:2560-2561, 2611-2613`). The deletion needs
   `Evaluator::delete` (I1) and is shared with I5; mark it if it is not done.
5. G6: `TOP_LEVEL_ASPECT_DRIVERS` (`execution_seam.rs:857-865`, read inside a function at `:218`)
   moves to per-command state.
6. Integrate the registration changes the other groups request in `skyframe_executor.rs`:
   - group 3: `ConfigurationEnvironment` trimmed;
   - group 4: the `StarlarkTransitionCache` clear points;
   - group 2: the cells of `FetchSetup`;
   - group 5: the executor per server.

Hand back: one line per key. Per registration row of section B: what changed. Tests:
- `TargetPatternPhaseValue` prunes when only `non_expanded_labels` differs;
- a stale conflict node is not reused across two evaluations.

## Cross-group items (one owner each)

The Cargo dependencies that drive these: `blaze_sky_loading`, `blaze_bzlmod` and `blaze_build_config`
do not depend on each other. All three depend on `blaze_skyframe`. `blaze_sky_exec` depends on
`blaze_sky_loading` but not on `blaze_bzlmod`.

| # | Item | Groups touched | Recommended owner and order |
|---|---|---|---|
| X1 | **One precomputed type.** There are two `PrecomputedKey`/`PrecomputedValue` pairs (`crates/blaze_sky_loading/src/precomputed.rs:59`, `crates/blaze_bzlmod/src/skyframe/precomputed.rs:70`) with the same `PRECOMPUTED` name. That makes `starlark_semantics` two nodes. Both values hold `Arc<dyn Any>` with no equality, so every per-command injection would dirty all readers (Java prunes injected values by `PrecomputedValue.equals`, `J:.../skyframe/PrecomputedValue.java:128`). `blaze_build_config` cannot use either today, and it needs `STARLARK_SEMANTICS`, `PATH_PACKAGE_LOCATOR` and `BASELINE_*`. | 1, 2, 3, 4, 5, 6 | I recommend **one `Precomputed<T: PartialEq>` in `blaze_skyframe`** (the only crate all of them depend on), storing an `fn(&dyn Any, &dyn Any) -> bool` with each value. Owner: **group 1, first, before every other group's G1 work**, with I1's consent because the code lands in I1's crate. Alternative if I1 refuses: `blaze_bzlmod_model`, the other shared dependency, which is a worse fit. Each `T`'s `PartialEq` must be checked against its Java `equals` (list at `crates/blaze_sky_loading/src/precomputed.rs:124-157` and `crates/blaze_bzlmod/src/skyframe/precomputed.rs:150-190`). |
| X2 | **`CLIENT_ENVIRONMENT_VARIABLE` key placement.** `ClientEnvironmentKey` and `EnvironmentVariableValue` live in `blaze_bzlmod` (`precomputed.rs:213, 224`). ACTION_EXECUTION (group 5) needs them, and `blaze_sky_exec` cannot depend on `blaze_bzlmod`. Java: `J:.../skyframe/ClientEnvironmentFunction.java`. | 2, 5 | Move them with X1 into the same module. Owner: group 1 as part of X1. Then group 2 adapts and group 5 adds the dependencies. |
| X3 | **FileValue seams for crates that cannot name `FileKey`.** `blaze_bzlmod` (5 G2 sites) and `blaze_build_config` (1 site) need `FILE` dependencies. | 2, 3 | Each crate gets its own small lookup trait (the existing pattern: `PackageLookup`, `MainRepositoryMappingLookup`), implemented in `blaze_repo_rules/src/setup.rs` (group 2) and in `blaze_sky_top` (group 3 asks group 6). Groups 2 and 3 work in parallel. Moving `FileKey` into a lower crate would be a redesign; I advise against it. |
| X4 | **`Artifact.generating_action_key: OnceLock`** (`crates/blaze_actions/src/actions/artifact.rs:66`, plan 2.5 G4). | 4, 5, 6 | Resolved by the survey: it is set before the value is returned (`rule/actions_util.rs:215`, `configured_aspect.rs:252`, `analysis_services.rs:166-168`, `blaze_sky_top/src/action_template_expansion_function.rs:262`), as Java's `setGeneratingActionKey` is. No change. Group 4 records the evidence. Owner: group 4. |
| X5 | **`PackageDeclarations` / `Package` mutation after return** (plan 2.5 G4). | 1, 4 | Resolved by the survey: no write after return. `set_contains_errors` and `set_failure_detail` (`crates/blaze_packages/src/package/package.rs:385, 394`) are unguarded but are only called inside `finish_build*` and during rule construction (`Rule.java:650-656` has the same restriction). Owner: group 1. Optional guard (S4). |
| X6 | **Shared value types in lower crates**: `TreeArtifactValue` and `RunfilesArtifactValue` (`blaze_exec`), `InterimModule` (`blaze_bzlmod_model`), `MacroInstance`/`AttributeContainer`/`Target` (`blaze_packages`), `OpaquePlatformInfo` (`blaze_build_config`, made in `blaze_sky_analysis`). | 1, 2, 3, 4, 5 | `blaze_exec` goes to group 5, `blaze_bzlmod_model` to group 2, `blaze_packages` to group 1, `OpaquePlatformInfo` to group 3 (with group 4 for `platform_function.rs`). No ordering between them. |
| X7 | **Address-keyed memos and pinned heaps**: `KEEP_ALIVE` (`bzl_load.rs:732`), `CONVERTED` (`blaze_analysis/src/config/transitions.rs:897`), `MEMO`/`FROZEN_TO_SET`/`KEPT_HEAPS` (`blaze_analysis/src/rule/depsets.rs:70, 74, 122`), `GENERATOR_NAME_HEAPS`, `BY_FILENAME` (write-only, see the correction in group 2), `ACTION_INPUT_SETS` (`blaze_exec/src/analysis/actions/mod.rs:268`). | 1, 4, 5 | They are correct as long as no heap is freed, and they grow without bound. This is a root decision (plan 3.3), not I2 work. I2 workers must not free heaps. |
| X8 | **`blaze_starlark` `SOURCE_READER`** (`crates/blaze_starlark/src/error.rs:360`, read at `:383`): source lines are read from disk while an error is converted inside a function, so an error value stored in the graph carries file text without a FILE dependency. Java formats at print time (`J:net/starlark/java/eval/EvalException.java:228`). | 1 (and every function that converts `EvalError`) | Owner: group 1 (BZL_LOAD and PACKAGE are the main producers). Option: format the source line at print time, as Java does. It needs root to name `blaze_starlark` as an editable crate. |
| X9 | **`skyframe_executor.rs` registration edits** (`crates/blaze_sky_top/src/skyframe_executor.rs:217-522`). | all | Owner: group 6, last. The other groups send their changes as a list. |
| X10 | **Per-command reset points** (the `preparePackageLoading` port, `beforeCommand`/`afterCommand`): `bzl_compile_cache`, `PACKAGE_PROGRESS`, repository cells, executor per-build state. | 1, 2, 5, 6 | Each group adds the reset method on its own type. Wiring the calls is I3's (`sync`), so the groups do not invent call sites. |

## Totals

Per group (84 keys):

| Group | Keys | Distinct value types | EQ-OK | EQ-WRITE | IDENTITY | RUST-STRONGER | MIXED |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 `blaze_sky_loading` | 32 | 31 (`EmptySkyValue` serves 2 keys) | 11 | 7 | 8 | 4 | 2 |
| 2 `blaze_bzlmod` 13 + `blaze_repo_rules` 6 | 19 | 17 (`SingleExtensionValue` and `EnvironmentVariableValue` serve 2 keys each) | 12 | 3 | 1 | 1 | 2 |
| 3 `blaze_build_config` | 7 | 7 | 5 | 0 | 1 | 0 | 1 |
| 4 `blaze_analysis` 7 + `blaze_sky_analysis` 3 | 10 | 10 | 5 | 0 | 5 | 0 | 0 |
| 5 `blaze_sky_exec` | 9 | 9 | 1 | 5 | 2 | 1 | 0 |
| 6 `blaze_sky_top` | 6 | 5 (`ActionLookupConflictFindingValue` serves 2) | 0 | 3 | 3 | 0 | 0 |
| `blaze_skyframe` (I1) | 1 | 1 | 0 | 0 | 1 | 0 | 0 |
| **Total** | **84** | **80** | **34** | **18** | **21** | **6** | **5** |

The two precomputed value types are counted separately; after X1 there are 79.

Hermeticity, per the Java creation sites:
- NonHermetic: FILE_STATE, DIRECTORY_LISTING_STATE, PRECOMPUTED ×2, REPOSITORY_DIRECTORY,
  CLIENT_ENVIRONMENT_VARIABLE, REPOSITORY_ENVIRONMENT_VARIABLE, MODULE_FILE, REGISTRY, REPO_SPEC,
  YANKED_VERSIONS, SINGLE_EXTENSION_EVAL, BASELINE_OPTIONS, BUILD_DRIVER, TARGET_PATTERN_PHASE,
  ERROR_TRANSIENCE. That is 16 keys over 15 names.
- SemiHermetic: FILE, TARGET_PATTERN, ACTION_EXECUTION (3).
- Hermetic: the other 65.

Every Rust FUNCTION string has a Java creation site. None is a swap: `TestExpansionKey`→`TESTS_IN_SUITE`
and `TestsForTargetPatternKey`→`TEST_SUITE_EXPANSION` are Java's own pairings, and the conflict-finding
strings are `..._DETECTION` as in Java.

**Comparison with the plan (76 keys, 72 value types; 37 / 13 / 8 / 18).** The plan's tree was `78de6e8`
plus uncommitted work. At commit `78de6e8` the same grep gives 74 production keys, and the plan's 76
included `BazelModTidyKey` and `BazelFetchAllKey`, which were uncommitted then. Diffing the key list
of `78de6e8` against today's tree explains every difference.

Key changes:
- 7 keys added:
  - `ParsedFlagsKey` (build_config): EQ-OK;
  - `ActionTemplateExpansionKey` (sky_exec): IDENTITY;
  - five package-piece keys (`PackagePieceForBuildFileKey`, `EvalMacroKey`: IDENTITY;
    `PackageDeclarationsKey`, `MacroInstanceKey`, `NonFinalizerPackagePiecesKey`: EQ-WRITE);
  - `ProjectKey` (sky_loading): RUST-STRONGER.

  That is 8 keys in total.
- 1 key moved: `ProjectFilesLookupKey` moved from `blaze_sky_top/src/project.rs` to
  `blaze_sky_loading/src/project_function.rs`, so sky_top goes from 7 to 6 and sky_loading from 25 to 32.

Key and value-type counts: 76 + 8 = **84** keys, and 72 + 8 new value types = **80** value types.

Verdicts on the 76 keys the plan knew, mine against the plan's:

| Verdict | Plan | Mine |
|---|---:|---:|
| EQ-OK | 37 | 33 |
| EQ-WRITE (EQ-DERIVABLE + EQ-HARD) | 13 | 15 |
| IDENTITY | 18 | 18 |
| RUST-STRONGER | 7 | 5 |
| MIXED | 1 | 5 |

The reclassifications:
1. `ModuleFileKey` and `BazelModuleResolutionKey`: EQ-OK becomes **MIXED**. `InterimModule::eq`
   compares the registry by URL, while Java uses identity (`Registry` is NotComparable). With a kept
   graph this prunes values that still hold the old registry object.
2. `LocalRepositoryLookupKey`: EQ-OK becomes **EQ-WRITE**. Java `Successful.equals` ignores `path`
   (`LocalRepositoryLookupValue.java:174-180`).
3. `TargetPatternPhaseKey`: EQ-OK becomes **EQ-WRITE**. Java `equals` ignores `nonExpandedLabels`
   (`TargetPatternPhaseValue.java:118-128`).
4. `PackageLookupKey` and `ContainingPackageLookupKey`: the plan counted each as RUST-STRONGER "for one
   variant". I count them as **MIXED**, because the other variants agree with Java and a hand-written
   `values_equal` lets them prune.

With these, 37 − 4 = 33, 13 + 2 = 15, 7 − 2 = 5, and 1 + 4 = 5 (the fifth MIXED is `PlatformValue`, as
in the plan). The 8 new keys add +1 EQ-OK, +3 EQ-WRITE, +3 IDENTITY and +1 RUST-STRONGER, which gives
34 / 18 / 21 / 6 / 5. The plan's headline "37 can opt in as they are, 13 need equality written, 8 must
not use their `PartialEq`" becomes:
- **34** opt in as they are;
- **18** need equality written;
- **5** MIXED need a hand-written `values_equal`;
- **6** must not use their `PartialEq` (RepositoryDirectory, ArtifactNestedSet, RepoDefinition,
  RecursivePkg, TargetPattern, Project);
- **21** stay at the default.

G2 sites against the plan's "8 sites in 4 files":
- `module_file_function.rs`: root `MODULE.bazel` (now `:226-229`), includes (`:462-476`, the plan's
  466 and 475), patch files (`:1216-1220`), and a new one, the non-registry override module file
  (`:1021-1030`).
- The plan's "builtin package lookup `:675-706`" is **not a production site**. `setup.rs:555`
  installs the Skyframe-backed lookup, and the `Builtin*` stand-in serves the dev binary and tests
  only.
- `lock_file.rs:48`: unchanged.
- Platform mappings: now `crates/blaze_build_config/src/skyframe.rs:270` (the plan said `:221`).
- Source artifacts: now `crates/blaze_sky_exec/src/artifact_function.rs:440` with `:461` (the plan
  said `:199`).
- New, needing a decision: `skyframe_action_executor.rs:3448, 3455`, generated-input metadata taken
  from a stat instead of the graph.

So there are **7 production bypasses in 4 files plus 1 decision item**. The survey also found
missing-dependency inputs that are not file reads: `main_repo_mapping` and the workspace status in
CONFIGURED_TARGET and ASPECT, ACTION_EXECUTION's captured `client_env`, and `BUILD_ID`.

G1 table against the plan: all 7 plan rows were verified, and line numbers are updated in the group
sections. New rows:
- `show_loading_progress` and `repo_contents_cache_path` are copied by value (group 1);
- `last_registry_invalidation = now()` is wrong for a server (group 2);
- `registry_factory` snapshots `nonstrict_repo_env` (group 2);
- `main_repo_mapping` and `workspace_status` are missing edges (group 4);
- `SkyframeActionExecutor`: `client_env`, `per_build_file_cache` panics on reuse, and per-build fields
  (group 5);
- `BuildDriverFunction.key_to_posted_events` (group 5);
- `TOP_LEVEL_ASPECT_DRIVERS` static (group 6). The plan's "progress counter" at
  `execution_seam.rs:850` is now this guard.

Plan 3.1 says `BY_FILENAME` is read by `repository_module.rs:531, 603`; that is stale. The research
doc's "LOADED stale after the .bzl is edited" does not hold today, because the file is compiled in. The
real gap is that semantics are not in the key.

## What I did not check

- The Rust `PartialEq` of each type wrapped in a precomputed value against its Java `equals` (X1 lists
  them). `StarlarkSemantics` was not compared field by field.
- Deep equality below two levels:
  - group 2: `AttrValue`, `ModuleExtensionUsage`, `Version`, `ModuleKey`, and `FactValue::Float` NaN
    against `StarlarkFloat.equals`;
  - group 4: `ConfigMatchingProvider`, `ConstraintCollection`, `PlatformProperties`.
- Key-side `Eq`/`Hash` consistency with the Java key classes. Only the manual impls were noted. The plan's
  sub-agent covered this, and I did not repeat it.
- File system reads in helpers outside the six groups' crates that functions call: the non-Skyframe
  globber in `blaze_packages`, rule implementations in `blaze_rules`, and `ActionCacheChecker`
  internals in `blaze_exec`.
- Whether C++ module discovery in Rust produces anything Java's `ActionExecutionValue.getDiscoveredModules`
  would carry.
- Whether `Precomputed::set_on` has a production caller (grep shows only the two definitions, at
  `crates/blaze_sky_loading/src/precomputed.rs:112` and `crates/blaze_bzlmod/src/skyframe/precomputed.rs:132`;
  callers were not traced through macros).
- `CyclesReporter` deduper reset; the callers of `ActionKeyContext.clear` in Java beyond
  `BlazeRuntime.java:833`.
- The statics of plan 3.1 were not re-opened one by one, apart from those consulted inside functions,
  which are listed above.
- Nothing was compiled or run. All line numbers are from a tree that other agents are changing.
