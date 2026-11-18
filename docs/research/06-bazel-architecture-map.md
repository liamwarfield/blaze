# Bazel 9.3.2 architecture map for the Blaze port

Written 2026-11-17 against the pinned source (`~/.cache/blaze/src/bazel-9.3.2`, commit 5951e93c).
All line counts are `wc -l` of the pinned Java (comments and blank lines included). Java paths are
relative to `src/main/java/com/google/devtools/build/` unless they start with `src/`.

Targets: `build //...` on brotli; `test //:all //python:all` on re2; `query`, `cquery`, `aquery`,
`mod graph`, `info`, `version`, `clean`, `shutdown`.

How the crate plan was made (section 5): I parsed all 1,142 BUILD targets under `src/main/java`
and built a file-level dependency graph of the 3,037 Java files (imports plus same-package simple
names, with comments, strings and nested-class names filtered out). Files were assigned to
candidate crates by path, with explicit moves for known cycle breakers. A layering pass then
pulled small leaf classes down into the crate that needs them and pushed anything else up until
every edge pointed down. The result is acyclic by construction. The rules are in Appendix A. The
scripts were scratch tools and are not in the repo. If root wants a guarded mapping, they belong
in `scripts/`.

Where something is my judgement and not read from the code, it says so.

---

## 1. Command flow, end to end

### 1.1 Client (C++, `src/main/cpp`, 15,284 lines in total, about 7,500 of them Linux-relevant)

| File | Lines | Role |
| --- | --- | --- |
| `main.cc` → `blaze.cc` | 2,226 | `Main()`: parse startup options, find the workspace, compute the output base, extract the install base, start or reuse the server, run the command over gRPC, map the exit code. Also handles `--batch` and `shutdown`/`clean --expunge` follow-ups such as server kill and output-base deletion. |
| `option_processor.cc/.h`, `option_processor_unix.cc` | 766+214 | Splits argv into startup options, command and command args. Finds rc files (`%workspace%/.bazelrc`, `~/.bazelrc`, `/etc/bazel.bazelrc`, `--bazelrc`), `--enable_platform_specific_config`, and passes **rc contents to the server as `--rc_source=<path>` and `--default_override=<index>:<command>=<option>` args** (line ~694). The server expands `--config`, not the client. |
| `rc_file.cc/.h` | 335+116 | rc parsing: `import`, `try-import`, `%workspace%`, line continuation, quoting. Errors here are client errors with client text. |
| `startup_options.cc/.h`, `bazel_startup_options.cc` | 705+441+111 | Startup flags: `--output_base`, `--output_user_root`, `--max_idle_secs`, `--batch`, `--host_jvm_args`, `--install_base`, and so on. |
| `workspace_layout.cc` | 87 | Workspace detection (`MODULE.bazel`, `REPO.bazel`, `WORKSPACE`, `WORKSPACE.bazel`). |
| `blaze_util_posix.cc`, `blaze_util_linux.cc` | 904+235 | Process start (daemonize), pid files, the `server/` dir, locks (`lock` file under the output base), signals. |
| `archive_utils.cc` | 333 | Extracts the embedded install zip to `install/<md5>/`. |
| `server_process_info.cc`, `util/*` | — | md5 (the output base is `~/.cache/bazel/_bazel_$USER/<md5(workspace)>`), path and string utilities. |

Protocol: `src/main/protobuf/command_server.proto`, `service CommandServer { Run (stream
RunResponse), Cancel, UpdateTerminalSize, Ping }`. `RunRequest` carries the cookie, args (startup
args already folded in), client env, cwd and invocation policy. `RunResponse` streams stdout and
stderr chunks and finishes with the exit code. The server writes `server/command_port`,
`server/request_cookie` and `server/server.pid.txt` under the output base.

Recommendation (my judgement): for the first target, use a Rust client in its own crate
(`blaze_client`) that does the same path computation (md5 output base, install base) and then
calls the server code **in-process** for every command. In effect that is `--batch` with no
daemon. Section 3 assumes one command per process, which this gives directly. `shutdown` then just
succeeds, and `info server_pid` / `info server_log` print what a batch server would. A gRPC server
with Bazel's protocol can come later, because `blaze_runtime` already takes a request struct
equivalent to `RunRequest`.

### 1.2 Server

| Class | File | Lines | Role |
| --- | --- | --- | --- |
| `Bazel.main` | `lib/bazel/Bazel.java` | 140 | Lists `BAZEL_MODULES` (about 45 `BlazeModule`s, **order matters**: the last strategy provider wins), then calls `BlazeRuntime.main`. |
| `BlazeRuntime` | `lib/runtime/BlazeRuntime.java` | 1,948 | `main` (911) → `serverMain` (1200) or batch. Builds the runtime from modules (`BlazeModule.globalInit/blazeStartup/serverInit/workspaceInit`), creates `BlazeWorkspace` (`initWorkspace`, 271), `ConfiguredRuleClassProvider`, `BlazeCommandDispatcher`. |
| `GrpcServerImpl`, `CommandManager` | `lib/server/` | 787+288 | gRPC `Run` → dispatcher; idle shutdown, pid file watcher. |
| `BlazeCommandDispatcher` | `lib/runtime/BlazeCommandDispatcher.java` | 1,091 | `exec` (192) → `execExclusively` (363): command lock, output-base lock, `BlazeOptionHandler.parseOptions`, `CommandEnvironment` creation, `BlazeRuntime.beforeCommand` (636), module `beforeCommand`, `UiEventHandler` install, `command.exec(env, options)`, `afterCommand`, exit-code and crash handling. |
| `BlazeOptionHandler` | `lib/runtime/BlazeOptionHandler.java` | 775 | `parseOptions` (466): `parseArgsAndConfigs` (258) reads `--rc_source`/`--default_override` (`ClientOptions`), applies rc options per command inheritance chain (`common` → `build` → `test`…), **expands `--config`** through `ConfigExpander.expandConfigOptions` (`ConfigExpander.java`, 304; `:linux` from `--enable_platform_specific_config`), `parseStarlarkOptions` (417, `--//foo:flag` via `StarlarkOptionsParser`), invocation policy (`common/options/InvocationPolicyEnforcer`). |
| `CommandEnvironment` | `lib/runtime/CommandEnvironment.java` | 1,202 | Per-command state: reporter, event bus, options, `SkyframeExecutor` access, client env, working directory, `syncPackageLoading` (sets `PrecomputedValue`s). |
| `BlazeWorkspace`, `BlazeDirectories`, `ServerDirectories` | `lib/runtime/`, `lib/analysis/` | 509+~270 | Output base, execroot `<output_base>/execroot/_main`, install base. |
| Commands | `lib/runtime/commands/` | 7,231 | `BuildCommand` 112, `TestCommand` 284, `QueryCommand` 255 (+`QueryEnvironmentBasedCommand` 307), `CqueryCommand` 214, `AqueryCommand` 201, `InfoCommand` 354 (+`info/` items 1,699), `VersionCommand` 98, `CleanCommand` 331, `ShutdownCommand` 77. `mod`: `lib/bazel/commands/ModCommand.java` with `lib/bazel/bzlmod/modcommand/` (2,650). |

`version` and `shutdown` never touch Skyframe. `clean` deletes `execroot`/output dirs (or the whole
output base with `--expunge`, finished by the client) and resets Skyframe. `info` is mostly
directories and values, but `bazel-bin`, `bazel-genfiles` and `bazel-testlogs` need a
`BuildConfigurationValue`, and so the options → configuration path (§2.3).

### 1.3 Build (`lib/buildtool/`)

`BuildCommand.exec` → `BuildTool.processRequest` (`BuildTool.java:858`, 1,914 lines) →
`buildTargets` (257). **In Bazel 9, `--experimental_merged_skyframe_analysis_execution` defaults to
true** (`BuildRequestOptions.java:410`), so the path taken is
`buildTargetsWithMergedAnalysisExecution` (595), known as "Skymeld":

1. **Target pattern phase.** `AnalysisAndExecutionPhaseRunner` (355) /
   `AnalysisPhaseRunner.evaluateTargetPatterns` evaluates `TARGET_PATTERN_PHASE` (keep_going from
   the flag), which posts `TargetParsingCompleteEvent` and `LoadingPhaseCompleteEvent`. Before
   that, `BuildTool` computes the main repo mapping (the "Computing main repo mapping:" line), and
   that drives all of bzlmod resolution.
2. **Configuration.** `SkyframeExecutor.createConfigurations` turns the top-level `BuildOptions`
   into a `BuildConfigurationKey` (`BUILD_CONFIGURATION_KEY`, with platform mappings and
   `--platforms`) and then a `BuildConfigurationValue`.
3. **Analysis and execution together.** `SkyframeBuildView.analyzeAndExecuteTargets`
   (`skyframe/SkyframeBuildView.java`, 1,560) builds one `BuildDriverKey` per top-level target and
   aspect and evaluates them all at once. `BuildDriverFunction` (745) requests `CONFIGURED_TARGET`,
   checks action conflicts, then requests `TARGET_COMPLETION`/`TEST_COMPLETION`. Those pull
   `ACTION_EXECUTION` through `ARTIFACT`/`ARTIFACT_NESTED_SET`.
   `ExecutionTool` (1,079) sets up the executor, strategies, the action cache, the execroot
   symlink forest (`SymlinkForest`) and output directories, then `SkyframeBuilder` (306).
4. **Wrap-up.** `BuildResultPrinter` (415) prints "INFO: Found N targets…" and "Build completed
   successfully, N total actions". `OutputDirectoryLinksUtils` (393) creates the `bazel-bin`,
   `bazel-out`, `bazel-testlogs` and `bazel-<ws>` convenience symlinks.

`cquery` and `aquery` take the non-merged path: `BuildTool.buildTargets` with analysis only
(`AnalysisPhaseRunner.execute`, 101), then `PostAnalysisQueryProcessor` (245) →
`ConfiguredTargetQueryEnvironment` or `ActionGraphQueryEnvironment` (`lib/query2/cquery`,
`lib/query2/aquery`). aquery's proto and jsonproto output goes through
`skyframe/actiongraph/v2/ActionGraphDump` (1,329 in the package).

`query`: `QueryCommand` → `BlazeQueryEnvironment` (`lib/query2/query/`) with
`TargetPatternEvaluator` plus transitive package loading through `TRANSITIVE_TRAVERSAL` keys
(`Label.TRANSITIVE_TRAVERSAL`, `TransitiveTraversalFunction`). Output formatters are in
`lib/query2/query/output` (3,494). `SkyQueryEnvironment` is only used with `--universe_scope` /
`--order_output=no`-style options; the default path does not use it.

### 1.4 Execution of an action

`ActionExecutionFunction` (1,608) → `SkyframeActionExecutor` (2,044): action cache check
(`ActionCacheChecker` 880, `CompactPersistentActionCache` 1,173, stored in
`<output_base>/action_cache/`) → `Action.execute` → for spawn actions `SpawnStrategyResolver` →
`SpawnStrategyRegistry` (768) picks by mnemonic/flags. On Linux the default order puts
`linux-sandbox` first (brotli oracle: "53 processes: 10 internal, 43 linux-sandbox").
`AbstractSpawnStrategy` (409) → `LinuxSandboxedSpawnRunner` (560, `lib/sandbox/`) → runs the
install base's `linux-sandbox` binary through `lib/shell` (`Command`, `JavaSubprocessFactory`).
`process-wrapper` is used for local and non-sandboxed spawns (`LocalSpawnRunner` 557). The spawn
log (`--execution_log_*`, compared by the oracle as `execlog.json`) comes from `SpawnLogModule` +
`CompactSpawnLogContext` (809) / `ExpandedSpawnLogContext`.

### 1.5 Tests

`TestCommand.exec` (98) → `doTest` (124): sets up an `AggregatingTestListener` (347), runs the
build with test targets (`TEST_SUITE_EXPANSION` determines tests, `TEST_COMPLETION` runs them), then
`TerminalTestResultNotifier` (347) + `TestSummaryPrinter` (324) / `TestSummary` (696) print the
summary ("Executed N out of N tests…"). Each test is a `TestRunnerAction` (1,401,
`analysis/test/`) created by `TestActionBuilder` (520). It is executed by `StandaloneTestStrategy`
(902, `lib/exec/`) through `TestStrategy` (506). The spawn runs `@bazel_tools//tools/test:test-setup.sh`
under the sandbox with the runfiles tree (built by `SymlinkTreeAction` / the `build-runfiles`
binary, or in-process by `SymlinkTreeHelper` 285). `test.xml` comes from the test or is generated
by the follow-up spawn (`generate-xml.sh`). `TestXmlOutputParser` reads it. `TestResult` and
`TestAttempt` are posted on the event bus, and the listener and BEP consume them.

### 1.6 Events and UI

`Reporter`/`EventHandler` (`lib/events`, 1,590) for messages. The Guava `EventBus` carries
"postable" events: `BuildStartingEvent`, `TargetParsingCompleteEvent`, `ActionExecutedEvent`,
`TestResult`, and so on. Terminal output comes from `UiEventHandler` (1,168) +
`SkymeldUiStateTracker` (252)/`UiStateTracker` (1,426) in `lib/runtime`: "Loading:",
"Analyzing: N targets (P packages loaded, T targets configured)", the progress bar and the final
INFO lines. The oracle records stderr, so the INFO lines and error text matter. Progress lines are
time-dependent and need normalising. BEP: `BuildEventStreamer` (1,098) → transports
(`lib/buildeventstream/transports`, 731) for `--build_event_json_file`. **The oracle records
`bep.json`**, so BEP is a scored surface, though not on the critical path for build or test.

---

## 2. Skyframe functions for these builds

Registry: `SkyframeExecutor.skyFunctions()` (`lib/skyframe/SkyframeExecutor.java:760-962`, file
4,910 lines) plus `BazelRepositoryModule` (`lib/bazel/BazelRepositoryModule.java:241-264`) through
`WorkspaceBuilder.addSkyFunction`. Names: `lib/skyframe/SkyFunctions.java` (191 lines), plus
`Artifact.ARTIFACT`, `FileStateKey.FILE_STATE`, `Label.TRANSITIVE_TRAVERSAL`,
`TransitiveTargetKey.NAME`, `RepoDefinitionValue.REPO_DEFINITION`, and
`FILE_SYMLINK_{CYCLE,INFINITE_EXPANSION}_UNIQUENESS` (`lib/io`). The "used" column is my
expectation from reading the code paths. It should be confirmed by counting nodes per function on
the oracle (`bazel dump --skyframe=count` after a build), and that dump is worth adding to
`scripts/oracle`.

Paths below are under `lib/`. Function file lines are given; keys and values are usually in a
same-named `*Value.java` / `*Key.java`.

### 2.1 Environment and filesystem (layer 0)

| Name | Function (lines) | Key → Value | Used by |
| --- | --- | --- | --- |
| PRECOMPUTED | `skyframe/PrecomputedFunction` (33) | `PrecomputedValue.Key` → `PrecomputedValue` (injected: Starlark semantics, build id, `--override_module`, registries, lockfile mode, …) | all |
| CLIENT_ENVIRONMENT_VARIABLE | `ClientEnvironmentFunction` (76) | `ClientEnvironmentFunction.Key` → `ClientEnvironmentValue` | repo rules, action env |
| ACTION_ENVIRONMENT_VARIABLE | `ActionEnvironmentFunction` (113) | → `ClientEnvironmentValue` | `--action_env` |
| REPOSITORY_ENVIRONMENT_VARIABLE | `RepoEnvironmentFunction` (117) | → `EnvironmentVariableValue` | `--repo_env` |
| FILE_STATE | `FileStateFunction` (101) | `vfs/FileStateKey` (`RootedPath`) → `actions/FileStateValue` | all |
| FILE | `FileFunction` (406) | `FileKey` → `actions/FileValue` (symlink resolution) | all |
| FILE_SYMLINK_*_UNIQUENESS | `io/FileSymlink*UniquenessFunction` (83 each) | error dedup | symlink cycles |
| DIRECTORY_LISTING_STATE / DIRECTORY_LISTING | 86 / 68 | `RootedPath` → `DirectoryListingStateValue` / `DirectoryListingValue` | globs, `//...` |
| DIRECTORY_TREE_DIGEST | 149 | → `DirectoryTreeDigestValue` | repo rules watching dirs |

### 2.2 Bzlmod and repositories (layer 1)

| Name | Function (lines) | Key → Value | Deps |
| --- | --- | --- | --- |
| BAZEL_LOCK_FILE | `bazel/bzlmod/BazelLockFileFunction` (185) | singleton → `BazelLockFileValue` | FILE, PRECOMPUTED |
| MODULE_FILE | `ModuleFileFunction` (1,014) | `ModuleFileValue.Key` → `ModuleFileValue` (root: `RootModuleFileValue`) | FILE, REGISTRY, BZL_COMPILE-like evaluation of MODULE.bazel with `ModuleFileGlobals`, `include()` |
| REGISTRY | `RegistryFunction` (102) | `RegistryKey` → `Registry` (`IndexRegistry`) | downloads from BCR through the repository cache |
| REPO_SPEC | `RepoSpecFunction` (87) | `RepoSpecKey` → `RepoSpecValue` | REGISTRY (`source.json`) |
| YANKED_VERSIONS | `YankedVersionsFunction` (68) | → `YankedVersionsValue` | REGISTRY |
| VENDOR_FILE | `VendorFileFunction` (197) | → `VendorFileValue` | only with `--vendor_dir` |
| BAZEL_MODULE_RESOLUTION | `BazelModuleResolutionFunction` (398) | singleton → `BazelModuleResolutionValue` | MODULE_FILE (discovery), YANKED_VERSIONS, `Selection`/`Discovery` |
| BAZEL_DEP_GRAPH | `BazelDepGraphFunction` (255) | singleton → `BazelDepGraphValue` | resolution, lockfile |
| BAZEL_MODULE_INSPECTION | `BazelModuleInspectorFunction` (215) | → `BazelModuleInspectorValue` | `mod graph` |
| SINGLE_EXTENSION_USAGES | `SingleExtensionUsagesFunction` (67) | `ModuleExtensionId` → usages | BAZEL_DEP_GRAPH |
| SINGLE_EXTENSION_EVAL | `SingleExtensionEvalFunction` (519) | → `SingleExtensionValue` | BZL_LOAD of the extension, runs Starlark, lockfile facts, may fetch repos (`module_ctx.path`) |
| SINGLE_EXTENSION | `SingleExtensionFunction` (117) | → `SingleExtensionValue` | validates `use_repo` |
| MODULE_EXTENSION_REPO_MAPPING_ENTRIES | 83 | → entries | SINGLE_EXTENSION |
| REPOSITORY_MAPPING | `skyframe/RepositoryMappingFunction` (174) | `RepositoryName` → `RepositoryMappingValue` | BAZEL_DEP_GRAPH, extension entries |
| REPO_DEFINITION | `bazel/repository/RepoDefinitionFunction` (331) | `RepositoryName` → `RepoDefinitionValue` | BAZEL_DEP_GRAPH / SINGLE_EXTENSION, BZL_LOAD of the repo rule |
| REPOSITORY_DIRECTORY | `bazel/repository/RepositoryFetchFunction` (875) | `RepositoryName` → `rules/repository/RepositoryDirectoryValue` | REPO_DEFINITION, runs the repo rule (`StarlarkRepositoryContext` 4,432 lines in `repository/starlark`), marker files `external/@<name>.marker`, repo contents cache, downloader (`repository/downloader` 3,880), decompressors (2,295) |
| LOCAL_REPOSITORY_LOOKUP | 36 | → `LocalRepositoryLookupValue` | WORKSPACE-era; still registered |
| BAZEL_FETCH_ALL, BAZEL_MOD_TIDY | 111, 114 | | `fetch`, `mod tidy`: not needed |

Order: `BAZEL_LOCK_FILE` → `MODULE_FILE`(root) → {`REGISTRY`, `MODULE_FILE`(deps), `YANKED_VERSIONS`}
→ `BAZEL_MODULE_RESOLUTION` → `BAZEL_DEP_GRAPH` → `REPOSITORY_MAPPING`(main) →
`SINGLE_EXTENSION_USAGES` → `SINGLE_EXTENSION_EVAL` → `SINGLE_EXTENSION` →
`REPO_DEFINITION` → `REPOSITORY_DIRECTORY`. The lockfile is rewritten at the end of the command by
`BazelLockFileModule.afterCommand` (`bazel/bzlmod/BazelLockFileModule.java`), and its bytes are a
scored surface.

### 2.3 Starlark, packages and patterns (layer 2)

| Name | Function (lines) | Key → Value |
| --- | --- | --- |
| STARLARK_BUILTINS | `StarlarkBuiltinsFunction` (393) | singleton → `StarlarkBuiltinsValue` (`@_builtins` injection from `src/main/starlark/builtins_bzl`, 10,884 lines of Starlark, mostly cc) |
| BZL_COMPILE | `BzlCompileFunction` (311) | `BzlCompileValue.Key` → compiled program (parse+resolve; for us, a starlark-rust `AstModule`) |
| BZL_LOAD | `BzlLoadFunction` (1,680) | `BzlLoadValue.Key` (`keyForBuild`, `keyForBzlmod`, `keyForBuiltins`) → `BzlLoadValue` (frozen module); deps: BZL_COMPILE, STARLARK_BUILTINS, REPOSITORY_MAPPING, PACKAGE_LOOKUP / CONTAINING_PACKAGE_LOOKUP (label checks), recursive BZL_LOAD |
| REPO_FILE / REPO_PACKAGE_ARGS | `RepoFileFunction` (203) / `RepoPackageArgsFunction` (141) | `REPO.bazel` → package args |
| PACKAGE_LOOKUP | `PackageLookupFunction` (419) | `PackageIdentifier` → `PackageLookupValue` (BUILD / BUILD.bazel; external repo via REPOSITORY_DIRECTORY) |
| CONTAINING_PACKAGE_LOOKUP | 63 | → `ContainingPackageLookupValue` |
| IGNORED_SUBDIRECTORIES | 246 | → `IgnoredSubdirectoriesValue` (`.bazelignore`, `REPO.bazel` `ignore_directories`) |
| GLOB / GLOBS | `GlobFunction` (75) + `GlobsFunction` (218) | `GlobDescriptor` / `GlobsValue.Key` → matches |
| PACKAGE | `PackageFunction` (1,890) + `PackageFunctionWith{SingleGlobsDep,MultipleGlobDeps,outGlobDeps}` (219/412/98) | `PackageIdentifier` → `PackageValue`; runs BUILD through `packages/PackageFactory`; deps: PACKAGE_LOOKUP, BZL_LOAD, GLOB(S), REPO_PACKAGE_ARGS, STARLARK_BUILTINS |
| EVAL_MACRO, MACRO_INSTANCE, NON_FINALIZER_PACKAGE_PIECES | 415/93/119 | only with lazy symbolic-macro expansion (off by default): skip at first |
| PACKAGE_ERROR(_MESSAGE) | 103/46 | query error reporting |
| TARGET_PATTERN | `TargetPatternFunction` (182) | `TargetPatternValue.TargetPatternKey` → `TargetPatternValue` |
| COLLECT_PACKAGES_UNDER_DIRECTORY | 131 (+`ProcessPackageDirectory` 360) | for `//...`; DIRECTORY_LISTING, PACKAGE_LOOKUP, IGNORED_SUBDIRECTORIES |
| COLLECT_TARGETS_IN_PACKAGE, RECURSIVE_PKG, PREPARE_DEPS_OF_PATTERN(S), PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY | 78/117/357+150/89 | used by `query` paths and `//...` preparation |
| TEST_SUITE_EXPANSION / TESTS_IN_SUITE | `TestsForTargetPatternFunction` (126) / `TestExpansionFunction` (206) | `test` command |
| TARGET_PATTERN_PHASE | `TargetPatternPhaseFunction` (526) | `TargetPatternPhaseKey` → `TargetPatternPhaseValue` (top-level targets, test filtering) |
| TARGET_PATTERN_ERROR | 102 | error path |
| TRANSITIVE_TRAVERSAL | `TransitiveTraversalFunction` (191) | `Label` → `TransitiveTraversalValue` (`query` preloading) |
| TRANSITIVE_TARGET | 198 | rarely; legacy |
| PROJECT / PROJECT_FILES_LOOKUP | 528 / 132 | PROJECT.scl lookup; check on the oracle whether it is evaluated for these builds |

### 2.4 Configuration (layer 3)

| Name | Function (lines) | Key → Value |
| --- | --- | --- |
| PARSED_FLAGS | `skyframe/config/ParsedFlagsFunction` (177) | → `ParsedFlagsValue` (native and Starlark flags) |
| BASELINE_OPTIONS | `BaselineOptionsFunction` (124) | → `BaselineOptionsValue` (for the exec-config ST hash) |
| FLAG_SET | `FlagSetFunction` (719) | `--scl_config` / project flags; trivially empty here, but evaluated |
| BUILD_OPTIONS_SCOPE | `BuildOptionsScopeFunction` (243) | flag scoping |
| STARLARK_BUILD_SETTINGS_DETAILS | 309 | Starlark flag types and defaults (re2 transitions, rules_python flags) |
| PLATFORM_MAPPING | `PlatformMappingFunction` (365) | `PlatformMappingKey` → `PlatformMappingValue` |
| BUILD_CONFIGURATION_KEY | `BuildConfigurationKeyFunction` (162) | `BuildOptions` → `BuildConfigurationKey` (platform mappings applied) |
| BUILD_CONFIGURATION | `BuildConfigurationFunction` (182) | `BuildConfigurationKey` → `analysis/config/BuildConfigurationValue` (1,047): fragments, output directories, mnemonic |

### 2.5 Platforms and toolchains (layer 4; these need configured targets, so they interleave with 2.6)

| Name | Function (lines) | Key → Value |
| --- | --- | --- |
| PLATFORM | `analysis/platform/PlatformFunction` (133) | → `PlatformValue` (needs CONFIGURED_TARGET of the platform) |
| REGISTERED_EXECUTION_PLATFORMS | `skyframe/toolchains/` (301) | → value; BAZEL_DEP_GRAPH (`register_execution_platforms`), CONFIGURED_TARGET of platforms |
| REGISTERED_TOOLCHAINS | 280 | → value; CONFIGURED_TARGET of `toolchain()` targets (exec-config analysis) |
| SINGLE_TOOLCHAIN_RESOLUTION | 314 | per toolchain type |
| TOOLCHAIN_RESOLUTION | 432 | `ToolchainContextKey` → `UnloadedToolchainContext` |

### 2.6 Analysis (layer 5)

| Name | Function (lines) | Key → Value |
| --- | --- | --- |
| CONFIGURED_TARGET | `ConfiguredTargetFunction` (614) + `DependencyResolver` (918) + `analysis/producers/` state machines (4,984) | `ConfiguredTargetKey` (469) → `ConfiguredTargetValue` (`RuleConfiguredTargetValue`, …). Deps: PACKAGE, BUILD_CONFIGURATION, TOOLCHAIN_RESOLUTION, CONFIGURED_TARGET of deps after transitions (Starlark transitions use STARLARK_BUILD_SETTINGS_DETAILS and BUILD_CONFIGURATION_KEY), ASPECT, BZL_LOAD, STARLARK_BUILTINS, PLATFORM |
| ASPECT | `AspectFunction` (1,087) | `AspectKeyCreator.AspectKey` → `AspectValue` (rules_cc and rules_python attach aspects) |
| TOP_LEVEL_ASPECTS, LOAD_ASPECTS | 498, 133 | only with `--aspects` |
| ACTION_LOOKUP_CONFLICT_FINDING, TOP_LEVEL_… | 99, 84 | action conflict check under Skymeld |
| BUILD_INFO | `WorkspaceStatusFunction` (51) | workspace status action (`volatile-status.txt`, `stable-status.txt`) |
| COVERAGE_REPORT | 64 | skip |

### 2.7 Execution and tests (layer 6)

| Name | Function (lines) | Key → Value |
| --- | --- | --- |
| BUILD_DRIVER | `BuildDriverFunction` (745) | `BuildDriverKey` → `BuildDriverValue` (Skymeld top) |
| TARGET_COMPLETION / ASPECT_COMPLETION | `CompletionFunction` (489) + `TargetCompletor` (137) / `AspectCompletor` (114) | top-level outputs, output groups |
| TEST_COMPLETION | `TestCompletionFunction` (152) | `TestCompletionKey` → runs `TestRunnerAction` attempts |
| ARTIFACT | `ArtifactFunction` (651) | `Artifact` → `FileArtifactValue`/`TreeArtifactValue` (source files through FILE) |
| ARTIFACT_NESTED_SET | 167 | `collect/nestedset/ArtifactNestedSetKey` → batched input requests |
| ACTION_EXECUTION | `ActionExecutionFunction` (1,608) | `actions/ActionLookupData` → `ActionExecutionValue` |
| ACTION_TEMPLATE_EXPANSION | 259 | tree artifacts (rare here) |
| RECURSIVE_FILESYSTEM_TRAVERSAL | 774 | source directories and filesets (rare here) |

### 2.8 Dependency layering (who requests whom)

```
L0  PRECOMPUTED  *_ENVIRONMENT_VARIABLE  FILE_STATE→FILE  DIRECTORY_LISTING(_STATE)
L1  BAZEL_LOCK_FILE  MODULE_FILE  REGISTRY  REPO_SPEC  YANKED_VERSIONS
    → BAZEL_MODULE_RESOLUTION → BAZEL_DEP_GRAPH (→ BAZEL_MODULE_INSPECTION for `mod graph`)
L2  REPOSITORY_MAPPING ⇄ (needs BAZEL_DEP_GRAPH, SINGLE_EXTENSION*)
    STARLARK_BUILTINS → BZL_COMPILE → BZL_LOAD (→ REPOSITORY_MAPPING, PACKAGE_LOOKUP)
    SINGLE_EXTENSION_EVAL (→ BZL_LOAD, REPOSITORY_DIRECTORY) → SINGLE_EXTENSION
    REPO_DEFINITION (→ BZL_LOAD) → REPOSITORY_DIRECTORY (→ BZL_LOAD, FILE, CLIENT_ENV…)
L3  PACKAGE_LOOKUP (→ REPOSITORY_DIRECTORY) → PACKAGE (→ BZL_LOAD, GLOB(S), REPO_PACKAGE_ARGS)
    COLLECT_PACKAGES_UNDER_DIRECTORY → TARGET_PATTERN → TARGET_PATTERN_PHASE (→ TEST_SUITE_EXPANSION)
    [query: TRANSITIVE_TRAVERSAL → PACKAGE]
L4  PARSED_FLAGS  BASELINE_OPTIONS  FLAG_SET  PLATFORM_MAPPING  STARLARK_BUILD_SETTINGS_DETAILS
    → BUILD_CONFIGURATION_KEY → BUILD_CONFIGURATION
L5  CONFIGURED_TARGET ⇄ {PLATFORM, REGISTERED_EXECUTION_PLATFORMS, REGISTERED_TOOLCHAINS,
    SINGLE_TOOLCHAIN_RESOLUTION, TOOLCHAIN_RESOLUTION, ASPECT}   (mutually recursive through keys,
    acyclic as a node graph)
L6  BUILD_DRIVER → {CONFIGURED_TARGET, ACTION_LOOKUP_CONFLICT_*, TARGET_COMPLETION, TEST_COMPLETION}
    → ARTIFACT / ARTIFACT_NESTED_SET → ACTION_EXECUTION → (ARTIFACT of inputs, FILE for sources)
```

`mod graph` needs L0–L1. `query` needs L0–L3. `cquery` and `aquery` need L0–L5 (aquery also reads
action keys, so action construction must be exact). `build` needs everything, and `test` adds
TEST_COMPLETION.

---

## 3. Evaluator semantics, and root's `blaze_skyframe`

### 3.1 What Java does (`build/skyframe`, 17,832 lines)

- **Restart protocol.** `SkyFunction.compute` returns `null` when `env.getValue` returned null
  (`SkyFunction.java:45-90`, `LookupEnvironment`:178). `AbstractParallelEvaluator` (1,143)
  registers the new deps, enqueues them, and re-runs `compute` from scratch when they are done.
  `getValuesAndExceptions` asks for a batch of deps in parallel. `SkyKeyComputeState` (`:506`) and
  `state/Driver` (StateMachine, 1,225 lines in `skyframe/state`) exist only to keep work across
  restarts. ConfiguredTargetFunction's producers are StateMachines.
- **Errors.** A function throws `SkyFunctionException` (`Transience`). `getValueOrThrow(key,
  E.class)` rethrows a dependency's exception of type `E`. With **keep_going** every node is
  computed as far as possible, and parents see child exceptions through `getValueOrThrow`. Without
  keep_going, the first error stops new work, then `ParallelEvaluator.bubbleErrorUp` (`:290`)
  re-runs each parent on the path to a root **in error-bubbling mode** (`env.inErrorBubbling()`)
  so the parent can wrap the exception. The `EvaluationResult` then holds exactly that one bubbled
  error. Other roots have neither a value nor an error (`constructResult`, `:506`). `ErrorInfo`
  keeps a `NestedSet` of root causes.
- **Cycles.** These are found only when evaluation stalls: `SimpleCycleDetector` (553) DFS from
  each unfinished root in **dependency request order** gives `CycleInfo(pathToCycle, cycle)`
  (205). `CyclesReporter` (143) with domain reporters (`TargetCycleReporter`,
  `BzlLoadCycleReporter`, `BzlmodRepoCycleReporter`, `ActionArtifactCycleReporter`,
  `RegisteredToolchainsCycleReporter`…) prints the user text. That text is shell-test visible.
- **Events.** Events a function emits are stored on its node (`ValueWithMetadata` 253) and
  replayed (`EmittedEventState`) so a restarted function does not print twice. `SkyFunction.extractTag`
  supports `--output_filter`.
- **Incrementality.** `InvalidatingNodeVisitor`, dirty and change pruning, reverse deps,
  `DiffAwareness`, `FilesystemValueChecker`, versions, hermeticity, action rewinding
  (`Reset`): about 9.5k engine lines plus about 2.7k in `lib/skyframe`.
- **Interning and graph.** Keys are interned (`SkyKeyInterner`). `InMemoryGraphImpl` weak-interns
  target labels of loaded packages (a memory optimisation, and the only engine-to-`lib` back
  edge).

### 3.2 Preserve or simplify (one non-incremental command per process)

| Semantic | Verdict |
| --- | --- |
| getValue / getValueOrThrow / valuesMissing protocol | **Preserve the API** so functions port line by line |
| Re-running `compute` on missing deps | **Simplify away.** It is never observable except through side effects that Java itself dedups (events) or avoids (state). |
| keep_going vs no-keep-going results, and which error is reported | **Preserve.** Exit codes and the "ERROR: …" lines depend on them. |
| Exception wrapping by parents (bubbling) | **Preserve the effect.** Parents must get the child's exception and may wrap it. |
| Multiple root causes (`ErrorInfo.rootCauses`) | **Preserve** for keep_going: BEP `aborted`/`rootCauses` and "N root causes" text. |
| Cycle detection and **deterministic** reporting text | **Preserve** |
| Events emitted once, with Java's `--output_filter` tag | **Preserve.** Restart dedup is not needed. |
| Postable ordering on the event bus | **Approximately.** It is already nondeterministic in Java, and the oracle normalises. |
| Several `evaluate()` calls per command, each with its own `EvaluationContext` and with injected `PrecomputedValue`s between them | **Preserve.** `BuildTool` and `SkyframeExecutor` do this: patterns, configs, analysis, execution. |
| Incrementality, invalidation, versions, rewinding, serialization | **Drop** (`// PORT: skipped, incremental`) |
| Key interning | **Drop.** `Arc` plus value equality is enough. |
| Parallelism | **Keep** (`--jobs`, `--loading_phase_threads`), but output must not depend on it. |

### 3.3 Assessment of `crates/blaze_skyframe` (861 lines)

Root's design: callers block on dependencies. `get_value` computes a missing dependency inline or
waits for the thread that owns it, and it returns `None` only on error or cycle. Cycles are found
eagerly in a wait-for graph, there are no restarts, and a permit pool caps concurrency.

**This is the right design for this port, and I recommend keeping it.** The reasons:

1. Ported functions read like Java. `if env.values_missing() { return Ok(None) }` stays in place
   and just stops firing except on errors.
2. It matches Bazel's own newer direction for the hardest Java code. `StarlarkRepositoryContext`
   with `--experimental_worker_for_repo_fetching` (a worker thread that blocks), module extension
   evaluation, and the "PackageFunction re-runs BUILD files on missing globs" logic all exist to
   fake blocking. Repo rules and extensions run their Starlark exactly once, which is also what the
   repo-fetching worker mode guarantees.
3. `SkyKeyComputeState` and `state/Driver` StateMachines work unchanged, because a single `drive()`
   call finishes.
4. starlark-rust cannot suspend and resume an evaluation. A restart model would force re-running
   Starlark (as Java does for packages) or a worker-thread shim everywhere.

The alternative, a faithful restart engine (enqueue, re-run, dirty tracking), is about 5k lines
of engine port. It would make Starlark-calling functions either re-execute (slow, and starlark-rust
heaps would be rebuilt) or need the worker-thread trick that blocking gives for free. Its only
advantage is bit-for-bit evaluation order, and order is already nondeterministic in Java.

Changes I recommend to the current crate. Each one is something the Java does that the current
code does not:

1. **Per-call context and abort reset.** `EvaluationContext` is fixed at `build()`, and `aborting`
   is never cleared. `SkyframeExecutor` runs several evaluations per command with different
   `keep_going`. Make `evaluate(roots, ctx)` take the context, reset `aborting`, and **do not
   memoize `Aborted` results** across calls. Java re-evaluates nodes with transient errors
   (`ErrorTransienceValue`).
2. **No-keep-going result shape.** Roots that only saw `Aborted` must come back with *no value and
   no error*, and the single real error must be the one reported (`ParallelEvaluator.constructResult`).
   Otherwise Blaze will print "evaluation aborted…" where Bazel prints the real error.
3. **Deterministic cycle reports.** The wait-for-graph detection is fine for *detecting* (it is
   the only option without restarts), but the `CycleInfo` it builds depends on which thread closes
   the loop. Record each node's direct deps in request order (Java's `GroupedDeps`). Once a cycle
   is detected, fail the cycle's members and build the reported `CycleInfo` with a
   `SimpleCycleDetector`-style DFS from the top-level key over those recorded deps. The recorded
   deps are needed anyway for `SkyframeErrorProcessor` root-cause walks and for `WalkableGraph`,
   which query environments use (`SkyframeExecutor.getEvaluator().getExistingValue`, rdeps in
   cquery's transition resolver).
4. **Root causes as a set.** `ErrorInfo.root_cause: Option<SkyKey>` should become a
   `NestedSet`-like ordered set merged from children, to match `ErrorInfo.getRootCauses()`.
5. **`in_error_bubbling()`** should return false always, with a comment. In the blocking model
   parents see child errors during normal compute, which is equivalent.
6. **Thread growth.** `eval_keys` spawns up to `parallelism` scoped threads per batch, and blocked
   parents keep their threads. Nested batches (configured-target fan-out, `ARTIFACT_NESTED_SET`
   recursion) can create thousands of OS threads, each with a large reserved stack. Add a global
   cap on live threads (beyond it, evaluate the batch inline and sequentially) and a stack-depth
   guard. Inline recursion depth equals dependency-chain depth plus Starlark recursion. Use large
   reserved stacks (64–256 MiB of virtual memory) and, past a depth threshold, hand the key to a
   fresh thread.
7. **Interrupts.** Add a cancellation flag checked at `get_value` (Ctrl-C, and
   `--keep_going=false` abort of running siblings, which Java interrupts).
8. **Contract rule for PORTING.md:** never hold a lock across `get_value`. Java code never blocks
   inside `synchronized`, but a literal port might, and in a blocking engine that can deadlock.
   Also: Starlark heaps must not be held across a dependency request that can re-enter the same
   module, so only frozen values cross nodes.
9. `get_value` for a key with no registered function panics. Java treats that as a crash too, so
   this is fine, but put the Java text in the panic message.
10. Keep the engine free of `lib` types. The single Java back edge, label weak-interning in
    `InMemoryGraphImpl`, is a memory optimisation to drop.

---

## 4. Core types

| Type(s) | Java (lines) | Crate | Notes |
| --- | --- | --- | --- |
| `Label`, `PackageIdentifier`, `RepositoryName`, `RepositoryMapping`, `LabelParser`, `TargetPattern` | `lib/cmdline/` (877, 373, 406, 171, 233, 950) | `blaze_cmdline` | `PackageIdentifier` is also the PACKAGE SkyKey, and `Label` the TRANSITIVE_TRAVERSAL key. Because of Rust's orphan rule, `impl Key for PackageIdentifier` must live in `blaze_cmdline` (which should then depend on `blaze_skyframe`, as Java's cmdline does) or use newtype keys in the function crates. I recommend the dependency: it is Java's direction and costs nothing. Starlark `Label` value: `starlark_value` impl lives here, behind `blaze_starlark`. |
| `PathFragment`, `Path`, `Root`, `RootedPath`, `FileSystem`, `UnixGlob`, `Dirent`, `Symlinks` | `lib/vfs/` (991, 967, 347, 162, 898, 985) | `blaze_vfs` | `RootedPath` is the FILE_STATE key (`FileStateKey`): same orphan rule, so vfs depends on skyframe. `Fingerprint.addPath` becomes an extension trait in vfs (cycle break). |
| `Artifact` (Source/Derived/Tree/Special), `ArtifactRoot`, `ArtifactFactory`, `ArtifactOwner`, `ActionLookupKey`, `ActionLookupData` | `lib/actions/` (1,293, 310, 725, …) | `blaze_actions` | Derived roots: `bazel-out/<mnemonic>/{bin,genfiles,testlogs}`. Execroot is `<output_base>/execroot/_main`. External source roots are `external/<canonical repo name>` (`+`-separated canonical names in Bazel 9, for example `rules_cc+`). |
| Output layout | `analysis/config/OutputDirectories` (198), `OutputPathMnemonicComputer` (429) | `blaze_actions` (lives with `BuildConfigurationValue`, see §5) | Mnemonic `[cpu]-[compilation_mode][-platform_suffix][-ST-<hash>]`, for example `k8-fastbuild` (brotli) or `k8-opt-exec-ST-<12 hex>` for the exec config. The hash is `transitionDirectoryNameFragment` over the **option diff against the baseline** (`computeNameFragmentWithDiff`, `:289`). It therefore depends on **every registered FragmentOptions class's values**, including Android, Apple, ObjC, J2ObjC and Java options that we otherwise skip (see §6). |
| `BuildOptions`, `FragmentOptions`, `Fragment`, `FragmentClassSet`, transitions interfaces | `analysis/config/` (681, 165, 136, …) | `blaze_config` (below packages) | Java targets confirm the split: `analysis/config:build_options`, `:fragment` and `:fragment_options` are below `packages:packages`. `CoreOptions` (1,268) and its converters (which need `packages/BuildType`) are above packages. |
| `BuildConfigurationValue`, `CoreOptions`, `TestConfiguration`, `BuildConfigurationKey` | `analysis/config/BuildConfigurationValue` (1,047), `skyframe/config/BuildConfigurationKey` (122) | `blaze_actions` | Java's `build_configuration` target sits between `actions:artifacts` and `actions:actions`. Its checksum (hash of `BuildOptions`) appears in cquery and aquery output. |
| `Target`, `Rule`, `Package`, `Attribute`, `RuleClass`, `Type`/`BuildType`, `SelectorList` | `lib/packages/` (248, 1,015, 1,787, 2,490, 2,433, 860, 924) | `blaze_packages` | A 22k-line file-level SCC in Java. Keep it in one crate. Attribute storage is in `Rule`/`AttributeContainer` equivalents. `query --output=proto` attribute serialisation is in `query2/query/output`. |
| Providers / `Info`: `Provider`, `Info`, `NativeInfo`, `StarlarkInfo`, `StarlarkProvider`, `BuiltinProvider` | `lib/packages/` (67, …, 659, 157) | `blaze_packages` (with the `blaze_buildapi` trait) | `StarlarkInfo` must be a starlark-rust value. Field order and `repr` follow Java. |
| `ConfiguredTarget`, `RuleConfiguredTarget`, `TransitiveInfoProviderMap`, `RuleContext`, `StarlarkRuleContext` | `lib/analysis/` (153, 335, 244, 2,259, 1,315) | `blaze_analysis` | |
| `Action`, `AbstractAction`, `ActionKeyComputer`, `Fingerprint`, `SpawnAction`, `StarlarkAction`, `CustomCommandLine`, `ParameterFileWriteAction`, `CppCompileAction` | `lib/actions/` (239, 676, 68), `lib/util/Fingerprint` (349), `analysis/actions/` (7,842), `rules/cpp/CppCompileAction` (2,125) | `blaze_actions` / `blaze_analysis` / `blaze_rules` | **Action keys must be byte-exact** (aquery `actionKey`, the action cache, and the oracle's execlog). `Fingerprint` = MD5 or SHA-256 over Java's `addString` (varint length plus UTF-8) and `addInt` encodings. Port these exactly. |
| Action cache | `actions/cache/` (2,307: `CompactPersistentActionCache` 1,173, `ActionCache` 489), `ActionCacheChecker` (880) | `blaze_actions` | On-disk format only matters for reuse across runs. For non-incremental runs the behaviour (miss → execute → record) must still match the BEP and summary counts ("N processes: x internal…"). |
| `NestedSet`, `NestedSetBuilder`, `Order`, `Depset` | `lib/collect/nestedset/` (767, 391, 206, 633) | `blaze_collect` (`Depset` behind `blaze_starlark`) | Iteration order (STABLE/COMPILE/LINK/NAIVE_LINK) reaches command lines. Port exactly. |
| `Spawn`, `SpawnStrategy`, `SpawnRunner`, `SpawnStrategyRegistry`, `LocalSpawnRunner`, `LinuxSandboxedSpawnRunner`, `SandboxHelpers` | `lib/actions/` (193, 52), `lib/exec/` (318, 768), `exec/local` (557), `lib/sandbox` (560, 954) | `blaze_exec` | The `linux-sandbox` and `process-wrapper` binaries come from the oracle's install base until they are ported (decision 7 of `03-recommendations.md`). |
| Spawn log | `exec/SpawnLogContext` (272), `CompactSpawnLogContext` (809) | `blaze_exec` | A scored surface. |
| Runfiles | `analysis/Runfiles` (946), `RunfilesSupport` (722), `actions/RunfilesTree` (90), `SourceManifestAction` (471), `exec/SymlinkTreeHelper` (285) | `blaze_analysis` / `blaze_exec` | The manifest text format and `_repo_mapping` manifest (`RepoMappingManifestAction`) must match. pybind11 and Python tests need them. |
| Test running | `analysis/test/TestRunnerAction` (1,401), `exec/StandaloneTestStrategy` (902), `TestStrategy` (506) | `blaze_analysis` / `blaze_exec` | |

---

## 5. Crate layout

Root already has `blaze_util`, `blaze_options` (depends on util), `blaze_events`, `blaze_collect`,
`blaze_vfs`, `blaze_cmdline` (depends on vfs) and `blaze_skyframe` (no deps). The plan keeps those
names and their order.

### 5.1 Crates

Kept Java lines are from the layered assignment (Appendix A). They include small classes pulled in
from other packages. In total the kept crates hold **469,617 of 606,244 Java lines**; 136,627 are
excluded (§6).

| # | Crate | Java packages (main ones) | Kept lines |
| --- | --- | --- | --- |
| 1 | `blaze_util` | `lib/util` (minus path- and exec-dependent files), `lib/concurrent`, `lib/clock`, `lib/unsafe`, `lib/supplier`, `lib/bugreport`, `lib/profiler` (API; writer may stub), `lib/graph`, `lib/versioning`, `lib/util/regex`; Java and Guava shims | ~26,500 |
| 2 | `blaze_options` | `common/options` (+`processor`, `testing`) | 9,531 |
| 3 | `blaze_events` | `lib/events` | 1,590 |
| 4 | `blaze_starlark` | `lib/packages/semantics` (`BuildLanguageOptions`) + the starlark-rust bridge (thread context, semantics, printer, `Location`); see `05-starlark-rust.md` | 1,256 + new |
| 5 | `blaze_collect` | `lib/collect/nestedset`, `lib/collect` (compacthashset/compacthashmap, …) | ~5,400 |
| 6 | `blaze_skyframe` | `build/skyframe` (only ~3k worth of semantics needed; root's 861-line rewrite) | 18,023 (port ~4k) |
| 7 | `blaze_vfs` | `lib/vfs` (minus `inmemoryfs`, `OutputService`), `lib/unix`, `lib/io`, path-using `lib/util` files (`FileOutErr`, `DependencySet`, `PersistentMap`, …) | 14,461 |
| 8 | `blaze_cmdline` | `lib/cmdline`, `collect/SimpleTargetPatternMatcher` | 5,242 |
| 9 | `blaze_shell` | `lib/shell`, `util/Command*`, `DescribableExecutionUnit` | 3,753 |
| 10 | `blaze_buildapi` | `lib/starlarkbuildapi/**` (Starlark API traits and signatures) + `BzlLoadValue`/`StarlarkBuiltinsValue`/`FileKey` pulled from `lib/skyframe` | 19,775 |
| 11 | `blaze_config` | `analysis/config` low slice (`BuildOptions`, `FragmentOptions`, `Fragment`, `FragmentClassSet`, `Scope`, transitions interfaces, `ToolchainTypeRequirement`, `ConfigMatchingProvider`), `analysis/platform` info types, `BlazeDirectories`/`ServerDirectories`; **all FragmentOptions classes**, including `remote/options`, Android/Apple/ObjC/Java options | 13,184 |
| 12 | `blaze_packages` | `lib/packages`, `pkgcache` (part), bzlmod value types needed by packages (`ModuleKey`, `Version`, `BazelDepGraphValue`) | 35,493 |
| 13 | `blaze_query_engine` | `lib/query2/engine` | 6,109 |
| 14 | `blaze_bzlmod_model` | `lib/bazel/bzlmod` data and globals (minus `*Function`), `bazel/repository/{downloader,cache}`, `authandtls` | 10,657 |
| 15 | `blaze_actions` | `lib/actions` (+`cache`), `CoreOptions`, `BuildConfigurationValue`, `OutputDirectories`, `OutputPathMnemonicComputer`, `TestConfiguration`, `TreeArtifactValue`, `ArtifactNestedSetKey`, `OutputService` | 38,938 |
| 16 | `blaze_bep` | `lib/buildeventstream` (+`transports`), `lib/causes` | 1,890 (rest pulled down) |
| 17 | `blaze_analysis` | `lib/analysis` (cluster, `starlark`, `actions`, `test`, `constraints`, `configuredtargets`), analysis keys and values from `lib/skyframe` (`ConfiguredTargetKey`, `AspectKeyCreator`, `ConfiguredTargetAndData`, toolchain contexts), exec interfaces it uses (`SpawnStrategyResolver`, `ExecutionOptions`) | 64,973 |
| 18 | `blaze_rules` | `lib/rules/{cpp,python,config,platform,genrule,test,repository}`, `lib/bazel/rules` | 30,294 |
| 19 | `blaze_sky_loading` | `lib/skyframe` filesystem, package, bzl, glob and pattern functions, `pkgcache`, `packages/producers`, `rules/repository` values, decompressors | 26,605 |
| 20 | `blaze_bzlmod` | `lib/bazel/bzlmod/*Function`, `modcommand`, `bazel/repository` fetch functions and Starlark repo context | 14,222 |
| 21 | `blaze_sky_analysis` | `lib/skyframe` configured-target, aspect, toolchain and config functions, `analysis/producers`, `PlatformFunction` | 18,565 |
| 22 | `blaze_exec` | `lib/exec` (+`local`), `lib/sandbox` (Linux), `lib/standalone`, `WorkerOptions` | 10,037 |
| 23 | `blaze_sky_exec` | `lib/skyframe` action, artifact, completion and build-driver functions, `actiongraph/v2` | 12,239 |
| 24 | `blaze_sky_top` | `SkyframeExecutor`, `SequencedSkyframeExecutor`, `SkyframeBuildView`, `SkyframeActionExecutor`, `SkyframeErrorProcessor`, `analysis/BuildView` | 13,510 |
| 25 | `blaze_query` | `lib/query2` (`query`, `cquery`, `aquery`, `output`, `common`) | 16,183 |
| 26 | `blaze_buildtool` | `lib/buildtool` | 3,363 (+4,935 currently pushed into runtime, see note) |
| 27 | `blaze_runtime` | `lib/runtime` (+`commands`, `commands/info`), `lib/server`, `lib/bazel` modules, `bazel/commands/ModCommand`, sandbox/standalone modules | 50,141 |
| 28 | `blaze_client` | port of `src/main/cpp` (Linux parts, ~7.5k C++) | new |
| 29 | `blaze_protos` | generated from `src/main/protobuf/*.proto` + `src/main/java/.../buildeventstream/proto` + remote-apis: `failure_details`, `command_line`, `build` (query proto), `analysis_v2` (aquery), `spawn` (exec log), `build_event_stream`, `invocation_policy`, `command_server`, `test_status` | generated |
| — | `blaze` (bin) | `Bazel.main` and the module list | small |

Note: some leaf files ended up one crate too high because the automatic pass pushed them up
(`buildtool` → `runtime` 4,935 lines; `bazel/repository/starlark` → `runtime` 3,648 lines, through
`ProcessWrapper` and `RepositoryRemoteExecutor`). Moving those two dependencies down puts the files
back where they belong. That is a mechanical fix when the crates are created.

### 5.2 Crate DAG (acyclic; each crate lists only direct deps on earlier crates)

```
blaze_util        → (none)                        blaze_protos → (none)
blaze_options     → util
blaze_events      → util
blaze_starlark    → options util
blaze_collect     → util starlark
blaze_skyframe    → (none)            (Java's engine uses util/events/nestedset; root's rewrite needs none of them)
blaze_vfs         → util options skyframe
blaze_cmdline     → util starlark skyframe vfs
blaze_shell       → util vfs cmdline
blaze_buildapi    → util starlark collect skyframe vfs cmdline
blaze_config      → util options collect skyframe vfs cmdline buildapi
blaze_packages    → … config buildapi cmdline
blaze_query_engine→ util skyframe cmdline config packages
blaze_bzlmod_model→ … packages config buildapi
blaze_actions     → … shell packages bzlmod_model config
blaze_bep         → options util vfs cmdline config actions
blaze_analysis    → … actions bep packages config shell
blaze_rules       → … analysis actions packages
blaze_sky_loading → … rules analysis bep actions bzlmod_model query_engine packages
blaze_bzlmod      → … sky_loading analysis actions bzlmod_model packages
blaze_sky_analysis→ … bzlmod sky_loading rules analysis
blaze_exec        → … sky_loading rules analysis bep actions shell
blaze_sky_exec    → … sky_loading rules analysis actions query_engine
blaze_sky_top     → … sky_analysis sky_exec bzlmod sky_loading
blaze_query       → … sky_top sky_exec sky_analysis sky_loading
blaze_buildtool   → … sky_top
blaze_runtime     → everything above;  blaze_client → protos (+ runtime in-process);  blaze (bin) → runtime client
```

Why `blaze_sky_loading` sits above `blaze_analysis` and `blaze_rules`: `BzlLoadFunction` needs the
`.bzl` environment, and `rule()`, `attr.*`, `provider()` and `aspect()` are implemented in
`analysis/starlark/StarlarkRuleClassFunctions` (2,294) and `StarlarkAttrModule` (1,259). Java has
the same shape (`skyframe_cluster` depends on `analysis_cluster`). So `query` needs those
`analysis` files ported, but not configured-target analysis.

### 5.3 Java cycles broken (directory level; Bazel's target graph itself is acyclic)

| Cycle | Break |
| --- | --- |
| `util` ↔ `vfs` (`Fingerprint.addPath`) | `Fingerprint` stays in util. Path methods become an extension trait in vfs. |
| `util` ↔ `options` (`StringEncoding`, `Pair`, `RegexUtil` used by options; `OptionsUtils`, `FakeOptions` use options) | Leaf helpers go to the lowest crate. `OptionsUtils` moves to vfs. |
| `vfs` → `actions` (`OutputService`, `LocalOutputService`) | Move to `blaze_actions`. |
| `cmdline`/`util`/`collect` → `skyframe` (`SkyFunctions` names, `DetailedException`) | `SkyFunctions` name constants go into `blaze_skyframe` (or each function crate). `DetailedException` goes to util. |
| `collect/nestedset` → actions/vfs (`ArtifactNestedSetKey`, `NestedSetFingerprintCache`, `DigestMap`) | Move to `blaze_actions`. |
| `skyframe` engine → `cmdline`/`packages` (`InMemoryGraphImpl` label interning) | Drop (memory optimisation). |
| `cmdline` → `packages` (`Label` uses `BuildLanguageOptions`) | `BuildLanguageOptions` moves into `blaze_starlark`, below cmdline. |
| `packages` ↔ `analysis` (`Fragment`, `TransitionFactory`, `ToolchainTypeRequirement`, `RuleDefinitionEnvironment`, `TransitiveInfoProvider`) | Follow Bazel's targets: the low config slice becomes `blaze_config`. |
| `actions` → `analysis` (`BuildConfigurationValue.RunfileSymlinksMode`) | Move the nested enum to actions. `BuildConfigurationValue` itself lives in `blaze_actions`. |
| `analysis` → `bzlmod` (`StarlarkGlobalsImpl` registers `ModuleFileGlobals`) | Injection: `blaze_bzlmod` registers MODULE.bazel globals at startup. |
| `analysis` → `lib/skyframe` keys and values; `lib/skyframe` loading → `bzlmod`, `rules/repository` values | Keys and values move down to the domain crate. Functions stay in `sky_*` crates. |
| `analysis` → `skyframe` executor (`BuildView`, `Project`) | Move up to `blaze_sky_top`. |
| `lib/skyframe` → `runtime`/`buildtool`/`query2` (option classes: `BuildRequestOptions`, `ExecutionOptions`, `KeepGoingOption`, `MemoryPressureOptions`; `QueryException`, `CqueryNode`, `StarlarkOptionsParser`, `ConfigFlagDefinitions`) | Pull these option and leaf classes down to the lowest crate that needs them. |
| `packages` → `query2` (`pkgcache/RecursivePackageProvider` → `QueryException`) | `blaze_query_engine` sits just above packages, and `pkgcache` goes to `sky_loading`. |
| `buildapi` → `packages` (`CcInfoApi`, `CcModuleApi` reference `StarlarkInfo`) | Those two API files go with `blaze_rules`. |

### 5.4 Waves

Recommendation (my judgement): create **all crate skeletons in wave 0**, each compiling with
`unported!` bodies, in DAG order. Behaviour waves then fill *files* in any crate whose deps exist.
That decouples the crate DAG, which must be acyclic, from the behaviour order, where `query` needs
some `analysis` files before any of `blaze_sky_analysis`.

| Wave | Work | Becomes testable against the oracle |
| --- | --- | --- |
| W0 | Skeletons of all crates. Finish `blaze_util`, `blaze_options` (all option classes, including excluded modules' options), `blaze_events`, `blaze_collect`, `blaze_vfs`, `blaze_cmdline`, `blaze_shell`, `blaze_skyframe` fixes (§3.3), `blaze_protos` | Unit tests; `help flags-as-proto` once the options are registered |
| W1 | `blaze_client` (output base md5, install base, rc parsing), `blaze_runtime` skeleton (`BlazeRuntime`, `BlazeCommandDispatcher`, `BlazeOptionHandler`, `ConfigExpander`, `CommonCommandOptions`, `UiEventHandler` basics), commands `version`, `shutdown`, `clean`, `info` (directory keys), `BlazeDirectories` | `version`, `info output_base/execution_root/workspace/release/...`, `clean`, `shutdown` exit codes; rc and `--config` expansion (through `canonicalize-flags` if ported) |
| W2 | `blaze_starlark` bridge, `blaze_buildapi` signatures for MODULE.bazel and repo rules, `blaze_bzlmod_model` (`ModuleFileGlobals`, `InterimModule`, `Selection`, `Discovery`, `IndexRegistry`, `DownloadManager`/`HttpDownloader`, repository cache), the needed `blaze_sky_loading` FS functions, `blaze_bzlmod` (MODULE_FILE, RESOLUTION, DEP_GRAPH, INSPECTION, LOCK_FILE), `modcommand` | **`mod graph`** (brotli, re2) and **`MODULE.bazel.lock` bytes** (resolution only; extensions come in W3) |
| W3 | `blaze_packages` (Package/Rule/Attribute/RuleClass/Type, `PackageFactory`, `StarlarkNativeModule`, globs), `analysis/starlark` rule-definition builtins (`rule()`, `attr`, `provider`, `aspect`, `config`), `STARLARK_BUILTINS` (@_builtins), BZL_LOAD, PACKAGE, target patterns, `REPO_DEFINITION`/`REPOSITORY_DIRECTORY` with Starlark repo rules and `module_ctx` extensions (rules_cc `cc_configure`, rules_python toolchains, pybind11), downloader and decompressors, `blaze_query_engine`, `blaze_query` (`query` + output formatters) | **`query //...`** label_kind / jsonproto / `deps()` on brotli and re2; lockfile with extension facts |
| W4 | `blaze_config` + `CoreOptions`/`BuildConfigurationValue`/fragments (Cpp, Python, Test, Platform, Proto, Java, Apple, Android options), PARSED_FLAGS, PLATFORM_MAPPING, BUILD_CONFIGURATION(_KEY), Starlark transitions incl. `builtin_exec_platforms.bzl` exec transition, `blaze_sky_analysis` (CONFIGURED_TARGET, producers, toolchains, PLATFORM, ASPECT), `blaze_analysis` (RuleContext, StarlarkRuleContext, providers, select/config_setting), `blaze_rules` (platform, config, cc Java internals `CcModule`/`CcStarlarkInternal`/`CcToolchainFeatures`/`CcToolchainVariables`, `PyBuiltins`), `cquery` | **`cquery`** label_kind / jsonproto (configuration checksums, `k8-fastbuild`, exec `ST-` hashes) |
| W5 | Action construction: `SpawnAction`, `StarlarkAction`, `CustomCommandLine`/`Args`, `FileWriteAction`, `SymlinkAction`, `CppCompileAction`(+builder), link actions, `ParameterFileWriteAction`, `SourceManifestAction`, action keys, `ActionGraphDump`, `aquery` | **`aquery`** jsonproto (action keys, argv, inputs, env) |
| W6 | `blaze_sky_exec` (BuildDriver, ActionExecution, Artifact, Completion), `blaze_sky_top`, `blaze_exec` (strategy registry, linux-sandbox and local runners, spawn log), action cache, `ExecutionTool`, `SymlinkForest`, convenience symlinks, `BuildResultPrinter`, `UiStateTracker` | **`build //...` on brotli**: exit code, output tree hashes, execlog, stderr INFO lines |
| W7 | Tests: `TestRunnerAction`, `StandaloneTestStrategy`, `TestCompletionFunction`, `TEST_SUITE_EXPANSION`, runfiles trees and repo mapping manifest, `AggregatingTestListener`/`TerminalTestResultNotifier`/`TestSummaryPrinter`, `test.xml`; re2 Python (rules_python hermetic interpreter, pybind11 `.so`) | **`test //:all //python:all` on re2**: 21/21 results, summary text, test.xml |
| W8 | `blaze_bep` (streamer, JSON transport, events), gRPC server mode | **`bep.json`** and the client/server tests |

---

## 6. What can be skipped, and what cannot

### 6.1 Excluded packages (first target: Linux, non-incremental, no remote execution)

Total **136,627 Java lines** of 606,244 (22.5%), plus the non-Linux C++ client files (~3,600:
`blaze_util_windows.cc` 1,479, `util/*_windows.cc` 1,386, darwin and bsd ~554).

| Package | Lines | Why safe |
| --- | --- | --- |
| `net/starlark/java/**` | 28,546 | replaced by starlark-rust (decision 2026-11-17) |
| `lib/remote/**` except `remote/options` | 30,695 | no remote cache or execution; options kept for flag parsing |
| `lib/skyframe/serialization/**` | 24,365 | remote analysis caching and AutoCodec only |
| `docgen/**` | 7,271 | doc generation |
| `lib/worker/**` except `WorkerOptions` | 6,887 | workers are used by Java/Kotlin rules, not cc or py |
| `lib/rules/java/**` except `JavaOptions`/`JavaConfiguration` | 5,898 | no Java targets (but see 6.2) |
| `lib/includescanning` | 3,917 | Google-internal include scanning, off in Bazel |
| `runtime/commands`: Run, Dump, PrintAction, Config, Coverage, License, Canonicalize | 3,961 | not target commands (`canonicalize-flags` is a cheap later add) |
| `lib/buildeventservice` | 3,293 | BES upload; local BEP files stay |
| `lib/metrics` | 2,332 | GC and memory metrics (BEP `BuildMetrics` memory fields need normalising anyway) |
| `lib/dynamic` | 2,070 | dynamic execution |
| `(rules/)starlarkdocextract` | 1,988 | `starlark_doc_extract` |
| `skyframe/rewinding` | 1,708 | action rewinding (lost inputs with remote) |
| `vfs/inmemoryfs` | 1,507 | test fixture: port later with the unit tests that need it |
| `starlarkdebug` | 1,506 | debugger |
| `sandbox` Darwin/Docker/Windows/Hardened | 1,500 | non-Linux |
| `rules/genquery` | 1,416 | not used by the corpus |
| `platform` (sleep prevention, suspension) | 928 | |
| `bazel/commands` fetch/vendor | 917 | |
| `windows` | 770 | |
| `rules/android`, `apple`, `objc`, `proto`, `extra`, `bazel/rules/java` (non-option files) | 2,299 | |
| `profiler/memory`, `runtime/mobileinstall`, `bazel/coverage`, `bazel/debug`, `skyfocus`, `network` | 2,853 | |

Also skippable *inside* kept crates for the first target (not counted above): engine incremental
machinery ~9,500 (root's rewrite already does this), diff awareness and `FilesystemValueChecker`
~2,700, `SkyQueryEnvironment` and parallel visitors ~3,000, C++ LTO/FDO/header-discovery ~1,900
(re2 builds with plain `fastbuild`; check that `parse_headers` and `layering_check` do not need
`HeaderDiscovery`), lazy macro expansion functions ~600.

### 6.2 Looks optional but is not

- **All `FragmentOptions` classes and fragments of registered rule sets** (Android, Apple, ObjC,
  J2ObjC, Java, Proto, Python, Cpp, Test, Coverage, Platform, Core). `BazelRuleClassProvider`
  registers them all (`BazelRuleClassProvider.java:327`). They shape the `BuildOptions` checksum
  shown by cquery and aquery, the exec-config `ST-` hash (computed from the option diff), and the
  exec transition in `@_builtins//:common/builtin_exec_platforms.bzl` (465 lines), which sets
  Java, Android and Apple options by name. Their option *declarations* (defaults and converters)
  are also needed so rc files and flags parse.
- **`remote/options` and other modules' option classes** (Remote, BES, Worker, Dynamic, Metrics,
  Sandbox). `bazel help`, rc parsing and invocation policy see every registered option. An rc line
  with a remote flag must parse.
- **The bzlmod lockfile** (`BazelLockFileFunction`, `BazelLockFileModule`, Gson adapters
  `GsonTypeAdapterUtil`, `LockfileModuleExtensionMetadata`). It is read at start and rewritten at
  the end of every command, and its bytes are scored.
- **Repository cache and downloader** (`repository/cache` 773, `downloader` 3,880, decompressors
  2,295). BCR module files, `source.json` and archives come through them. The shared repo cache
  (`~/.cache/blaze/repo-cache`) must be read in Bazel's layout (`content_addressable/sha256/<hash>/file`).
  The repo contents cache and the marker files decide whether repos are re-fetched.
- **Linux sandboxing.** 43 of brotli's 53 spawns are `linux-sandbox`. The spawn log and the
  "N processes: x internal, y linux-sandbox" line expose the strategy, and sandboxing changes which
  undeclared inputs are visible. Port `LinuxSandboxedSpawnRunner`, `SandboxHelpers` and the
  `sandbox/cgroups` options. The binary itself can come from the install base.
- **Action cache and action keys.** Action keys are printed by aquery and are in the exec log.
  The cache check also drives the "processes" summary. Non-incremental does not mean the cache can
  be skipped: Bazel writes it, and a second `build` in the same output base must report cache hits
  as Bazel does. The on-disk format can differ from Bazel's as long as Blaze and the oracle never
  share an output base.
- **Workspace status / BUILD_INFO** (`BazelWorkspaceStatusModule` 344). The workspace status action
  runs in every build ("10 internal" processes include it), and stamping inputs exist even without
  `--stamp`.
- **Skymeld.** It is the default. Event order ("Analyzing:" interleaved with execution) and
  `BuildDriverFunction` are the real build path. The legacy split path is still needed for
  cquery and aquery.
- **BEP.** Not needed for build or test correctness, and the test summary does not use it, but
  `bep.json` is a recorded oracle surface. Keep `BuildEvent` interfaces from W3 on, since many
  event classes implement them, and port transports in W8.
- **`@_builtins` injection.** `cc_common` and `cc_library` internals for rules_cc 0.2.x still route
  through builtins Starlark (10,884 lines, run by starlark-rust) and Java natives in `rules/cpp`
  (`CcStarlarkInternal` 1,298, `CcToolchainFeatures` 1,352, `CcToolchainVariables` 1,064,
  `CppCompileAction` 2,125). There is no shortcut through rules_cc alone.
- **`PyBuiltins` and `rules/python`** (1,111) for rules_python 1.7 internals.
- **Client path computation.** The output base md5, the install base md5 and `--output_user_root`
  layout appear in `info` and in every absolute path the oracle normaliser rewrites.

---

## Appendix A. Layering rules used for §5

First match wins. `ORDER` is low to high. `IGNORE a b` drops edges from files matching `a` to
files matching `b` (cycle breaks by trait or injection). `FAMILY c x,y` limits where files homed
in `c` may be pushed up. Files not under `src/main/java` are not covered. Crate names are the short
forms of §5.1 (`util`, `sky-loading`, `X-*` = excluded).

```
ORDER base protos options util starlark-glue nestedset skyframe vfs cmdline shell buildapi config packages query-engine bzlmod-model actions bep analysis rules sky-loading bzlmod sky-analysis exec sky-exec sky-top query buildtool runtime
IGNORE build/skyframe/InMemoryGraphImpl (cmdline|packages|lib/skyframe)/
IGNORE analysis/starlark/StarlarkGlobalsImpl bzlmod/ModuleFileGlobals
IGNORE lib/actions/ analysis/config/BuildConfigurationValue
IGNORE lib/util/Fingerprint lib/vfs/
FAMILY config config,actions,analysis
FAMILY bzlmod-model bzlmod-model,bzlmod
FAMILY query-engine query-engine,query
FAMILY sky-loading sky-loading,sky-analysis,sky-exec,sky-top
FAMILY sky-analysis sky-analysis,sky-exec,sky-top
FAMILY sky-exec sky-exec,sky-top
sky-exec     lib/skyframe/(Action*|Artifact*|CompletionFunction|TargetCompletor|AspectCompletor|TestCompletion*|TargetCompletionValue|AspectCompletionValue|FilesystemValueChecker|RecursiveFilesystemTraversal*|BuildDriver*|WorkspaceStatus*|CoverageReport*|TreeArtifactValue|SkyframeActionExecutor|ConsumedArtifactsTracker|TopLevelActionLookupConflictFindingFunction|ExclusiveTestBuildDriverValue).java, lib/skyframe/actiongraph/
sky-analysis lib/skyframe/(ConfiguredTargetFunction|AspectFunction|ToplevelStarlarkAspectFunction|LoadAspectsFunction|DependencyResolver|StarlarkAspectFactory|ConfiguredTargetEvaluationExceptions|PrerequisitePackageFunction|StarlarkBuildSettingsDetailsFunction), lib/skyframe/{config,toolchains}/, lib/analysis/producers/
analysis     lib/skyframe/(ConfiguredTargetKey|AspectKeyCreator|toolchains/{UnloadedToolchainContext,UnloadedToolchainContextImpl,PlatformLookupUtil,NoMatchingPlatformData,NoMatchingPlatformException,ToolchainContextUtil,ToolchainException,ToolchainContextKey}|config/PlatformMappingException|AspectCreationException|BaseTargetPrerequisitesSupplier|RuleConfiguredTargetValue|RemoteConfiguredTargetValue|AbstractConfiguredTargetValue|ConfiguredTargetAndData|BuildOptionsScopeFunction|BuildOptionsScopeValue|LoadAspectsKey|LoadAspectsValue|TopLevelStatusEvents|IncrementalArtifactConflictFinder|ArtifactConflictFinder|ConfiguredValueCreationException|PackageValue|PackageoidValue|PrecomputedValue).java, lib/rules/Alias.java
packages     lib/pkgcache/(PackageManager|PathPackageLocator).java
sky-top      lib/skyframe/(SkyframeExecutor|SequencedSkyframeExecutor|SkyframeBuildView|*ExecutorFactory|BazelSkyframeExecutorConstants|SkymeldModule|SkyframeTargetPatternEvaluator|SkyframeErrorProcessor|SkyframeActionExecutor).java, lib/analysis/(BuildView|Project|AnalysisOperationWatcher).java
config       lib/analysis/(config/[^output]|platform/|PlatformOptions|BlazeDirectories|ServerDirectories|TransitiveInfoProvider|RuleDefinitionEnvironment|test/(TestConfiguration|CoverageConfiguration|TestSharding*|TestTrimming*|TestResources*))
sky-loading  lib/packages/producers/, lib/pkgcache/
query-engine lib/query2/engine/
bzlmod       lib/bazel/bzlmod/*Function.java
bzlmod-model lib/bazel/bzlmod/ (not modcommand)
base         lib/util/(StringEncoding|Pair|RegexUtil|Markdown|OS).java, lib/util/regex/, lib/unsafe/
skyframe     lib/skyframe/SkyFunctions.java
util         lib/skyframe/DetailedException(s).java, lib/analysis/BlazeVersionInfo.java, lib/actions/(CommandLineItem|CommandLineExpansionException).java
vfs          lib/util/{LongVersionGetter,DependencySet,FileSystemLock,MapCodec,OptionsUtils,PathFragmentFilter,PersistentMap,TempPathGenerator,ResourceUsage}.java, lib/util/io/{AsynchronousMessageOutputStream,FileOutErr,FileWatcher,TimestampGranularityMonitor}.java, lib/collect/PathFragmentPrefixTrie.java, lib/testing/, lib/io/
starlark-glue lib/packages/semantics/
actions      lib/collect/nestedset/{ArtifactNestedSetKey,DigestMap,NestedSetFingerprintCache}.java, lib/vfs/{OutputService,LocalOutputService}.java, lib/util/{ResourceConverter,CpuResourceConverter,RamResourceConverter}.java
cmdline      lib/collect/SimpleTargetPatternMatcher.java
shell        lib/shell/, lib/util/{CommandUtils,CommandBuilder,CommandFailureUtils,DescribableExecutionUnit}.java
runtime      lib/outputfilter/, lib/profiler/{CommandProfilerModule,CollectLocalResourceUsage}, lib/remote/options/
bep          lib/causes/
X-*          (the §6.1 list; X-memprof = lib/profiler/memory/)
util         lib/collect/ (not nestedset), lib/{util,concurrent,clock,unsafe,supplier,bugreport,profiler,events,jni,versioning,graph}/
bzlmod       lib/repository/, lib/bazel/{bzlmod,repository}/
nestedset    lib/collect/nestedset/
options      common/options/
vfs          lib/{vfs,unix}/
skyframe     build/skyframe/
cmdline      lib/cmdline/
packages     lib/{packages,pkgcache}/
actions      lib/actions/
analysis     lib/analysis/
buildapi     lib/starlarkbuildapi/
rules        lib/rules/, lib/bazel/rules/
sky-loading  lib/skyframe/
exec         lib/{exec,sandbox,standalone,worker}/
query        lib/query2/
buildtool    lib/buildtool/
bep          lib/buildeventstream/
runtime      lib/{runtime,server,bazel,authandtls}/, lib/*.java
```

Pull-down threshold: a leaf class and the closure it needs moved down when that closure was at most
5,000 lines and contained no files from the foundation crates (base … skyframe). Otherwise the
depending file moved up.
