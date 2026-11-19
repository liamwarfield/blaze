

## Totals

<!-- BEGIN GENERATED TOTALS -->
By status. Methods include abstract ones. An abstract method also counts as marked when the method of the same name in a direct subtype is marked (impl), or when it is a parameterless accessor of an @AutoValue/@AutoOneOf class whose type a marker names (type).

| status | files | lines | methods | methods marked |
| --- | --- | --- | --- | --- |
| PORTED | 1309 | 204565 | 15509 | 12969 |
| PARTIAL | 630 | 105754 | 9111 | 2265 |
| UNTOUCHED | 1098 | 95803 | 8959 | 0 |
| total | 3037 | 406122 | 33579 | 15234 |

Not PORTED, by reason:

| reason | status | files | lines | methods | methods unmarked | unported lines (est.) |
| --- | --- | --- | --- | --- | --- | --- |
| NEEDED | PARTIAL | 564 | 91819 | 7960 | 5858 | 67358 |
| NEEDED | UNTOUCHED | 771 | 76018 | 7014 | 7014 | 76018 |
| GENERATED_OR_TRIVIAL | PARTIAL | 11 | 142 | 34 | 28 | 0 |
| GENERATED_OR_TRIVIAL | UNTOUCHED | 121 | 1137 | 162 | 162 | 0 |
| TEST_SUPPORT | PARTIAL | 2 | 231 | 24 | 23 | 0 |
| TEST_SUPPORT | UNTOUCHED | 27 | 1956 | 220 | 220 | 0 |
| JVM_ONLY | PARTIAL | 4 | 478 | 34 | 27 | 0 |
| JVM_ONLY | UNTOUCHED | 41 | 4174 | 345 | 345 | 0 |
| OUT_OF_SCOPE_OS | PARTIAL | 5 | 527 | 37 | 34 | 0 |
| OUT_OF_SCOPE_OS | UNTOUCHED | 27 | 1855 | 205 | 205 | 0 |
| OUT_OF_SCOPE_BY_CONTRACT | PARTIAL | 6 | 987 | 67 | 56 | 0 |
| OUT_OF_SCOPE_BY_CONTRACT | UNTOUCHED | 54 | 5148 | 397 | 397 | 0 |
| STARLARK_RUST_DECISION | PARTIAL | 38 | 11570 | 955 | 820 | 0 |
| STARLARK_RUST_DECISION | UNTOUCHED | 57 | 5515 | 616 | 616 | 0 |
<!-- END GENERATED TOTALS -->


## Markers

<!-- BEGIN GENERATED MARKERS -->
| marker resolves to | markers | share |
| --- | --- | --- |
| method: name and line agree | 12678 | 52.2% |
| method: by name (line elsewhere) | 3907 | 16.1% |
| test: names a src/test/java class (no path) | 2013 | 8.3% |
| non-Java: C++/native/proto source | 1516 | 6.2% |
| field/other: identifier in file | 1356 | 5.6% |
| type: names a class | 1112 | 4.6% |
| test: names a src/test/java file | 522 | 2.1% |
| pathless: method: by name (no line) | 375 | 1.5% |
| library: JDK, protobuf, gRPC or other non-Bazel source | 313 | 1.3% |
| library or prose: no path, not a Bazel class (JDK, Guava, ...) | 96 | 0.4% |
| method: by name (no line) | 86 | 0.4% |
| file-level: no member, line outside methods | 80 | 0.3% |
| field/other: identifier in file (line inside a method) | 51 | 0.2% |
| pathless: field/other: identifier in file | 45 | 0.2% |
| type: names a class (line inside a method) | 45 | 0.2% |
| method: by line (no member named) | 27 | 0.1% |
| file-level: path followed by prose | 14 | 0.1% |
| stale path (file moved), by file name: method: by name (no line) | 14 | 0.1% |
| type: implicit constructor (class declares none) | 11 | 0.0% |
| method: by line only (name not found: renamed or wrong) | 8 | 0.0% |
| stale path (file moved), by file name: UNRESOLVED: name not in file, line not in a method | 2 | 0.0% |
| pathless: AMBIGUOUS (several files define it) | 1 | 0.0% |
| pathless: type: names a class | 1 | 0.0% |
| stale path (file moved), by file name: field/other: identifier in file | 1 | 0.0% |
| UNRESOLVED: no path, member not found | 17 | 0.1% |
| UNRESOLVED: name not in file, line not in a method | 13 | 0.1% |
| UNRESOLVED: Bazel file not found | 1 | 0.0% |
| total | 24305 | 100% |

`unported!` sites: 812; attributed to a Java file by class name: 448 (0 of them by a class name that several files define: first file taken); naming a Java test class: 272; not attributable: 92.
`// PORT: skipped` comments: 176; attributed to a Java file: 69.

<!-- END GENERATED MARKERS -->


## Packages

<!-- BEGIN GENERATED PACKAGES -->
| package | files | lines | PORTED | PARTIAL | UNTOUCHED | methods marked | NEEDED files not ported | NEEDED unported lines (est.) | lines out of scope | unported! sites |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| lib/skyframe | 501 | 65398 | 109 | 106 | 286 | 1274/5301 | 326 | 37851 | 4260 | 57 |
| lib/runtime | 159 | 23876 | 86 | 42 | 31 | 740/1503 | 62 | 9229 | 806 | 25 |
| lib/analysis | 337 | 51641 | 213 | 75 | 49 | 3057/4472 | 110 | 9035 | 229 | 99 |
| lib/rules | 150 | 27461 | 85 | 23 | 42 | 1091/1974 | 60 | 8747 | 111 | 23 |
| build/skyframe | 91 | 10545 | 11 | 28 | 52 | 170/1152 | 71 | 8080 | 123 | 7 |
| lib/remote | 131 | 23151 | 78 | 15 | 38 | 951/1586 | 44 | 6605 | 221 | 41 |
| lib/actions | 151 | 15528 | 72 | 44 | 35 | 951/1986 | 67 | 6016 | 123 | 30 |
| lib/query2 | 136 | 15759 | 92 | 18 | 26 | 794/1239 | 43 | 5330 | 6 | 11 |
| lib/bazel | 202 | 28659 | 130 | 59 | 13 | 1098/1585 | 71 | 4633 | 13 | 31 |
| lib/worker | 31 | 4912 | 1 | 6 | 24 | 14/372 | 28 | 4445 | 25 | 2 |
| lib/packages | 162 | 24699 | 111 | 29 | 22 | 1784/2589 | 47 | 3916 | 31 | 38 |
| lib/util | 105 | 7736 | 42 | 22 | 41 | 359/909 | 57 | 3912 | 194 | 8 |
| lib/starlarkbuildapi | 116 | 13851 | 59 | 9 | 48 | 385/628 | 35 | 3636 | 312 | 0 |
| lib/collect | 27 | 4204 | 5 | 5 | 17 | 104/465 | 21 | 3091 | 8 | 0 |
| lib/vfs | 55 | 6793 | 16 | 16 | 23 | 408/943 | 35 | 2996 | 180 | 5 |
| lib/profiler | 32 | 3781 | 2 | 5 | 25 | 12/303 | 22 | 2946 | 684 | 0 |
| lib/buildtool | 45 | 7098 | 19 | 9 | 17 | 170/371 | 22 | 2900 | 288 | 8 |
| lib/concurrent | 33 | 3000 | 0 | 3 | 30 | 9/326 | 29 | 2804 | 102 | 0 |
| lib/includescanning | 14 | 2714 | 1 | 1 | 12 | 2/165 | 12 | 2623 | 6 | 0 |
| lib/buildeventservice | 9 | 2542 | 1 | 2 | 6 | 10/164 | 8 | 2158 | 0 | 3 |
| lib/sandbox | 42 | 5599 | 15 | 8 | 19 | 147/363 | 20 | 1695 | 552 | 4 |
| lib/server | 13 | 1423 | 1 | 2 | 10 | 5/112 | 11 | 1314 | 44 | 0 |
| common/options | 47 | 5740 | 26 | 5 | 16 | 313/534 | 13 | 1059 | 737 | 4 |
| lib/starlarkdocextract | 8 | 1030 | 0 | 0 | 8 | 0/70 | 8 | 1030 | 0 | 0 |
| lib/dynamic | 9 | 1455 | 2 | 1 | 6 | 13/73 | 7 | 1004 | 0 | 2 |
| lib/starlarkdebug | 10 | 1059 | 1 | 1 | 8 | 2/81 | 9 | 991 | 0 | 1 |
| lib/exec | 46 | 6714 | 28 | 9 | 9 | 270/422 | 15 | 834 | 250 | 6 |
| lib/authandtls | 19 | 1658 | 5 | 8 | 6 | 52/131 | 14 | 777 | 0 | 6 |
| lib/graph | 9 | 727 | 0 | 0 | 9 | 0/85 | 7 | 715 | 12 | 0 |
| lib/metrics | 11 | 1626 | 3 | 2 | 6 | 47/144 | 6 | 674 | 253 | 2 |
| lib/pkgcache | 24 | 1645 | 11 | 7 | 6 | 74/147 | 10 | 532 | 28 | 6 |
| lib/bugreport | 4 | 461 | 0 | 2 | 2 | 11/58 | 4 | 338 | 0 | 2 |
| lib/cmdline | 21 | 2992 | 14 | 5 | 2 | 252/357 | 6 | 332 | 5 | 1 |
| lib/shell | 19 | 1804 | 12 | 5 | 2 | 126/227 | 5 | 327 | 368 | 0 |
| lib/io | 8 | 260 | 2 | 6 | 0 | 14/37 | 6 | 145 | 0 | 0 |
| lib/unix | 6 | 616 | 3 | 0 | 3 | 41/86 | 3 | 136 | 0 | 1 |
| lib/causes | 5 | 185 | 2 | 3 | 0 | 9/24 | 3 | 116 | 0 | 1 |
| lib/buildeventstream | 31 | 1688 | 24 | 3 | 4 | 130/167 | 6 | 107 | 4 | 1 |
| lib/supplier | 4 | 83 | 0 | 0 | 4 | 0/12 | 2 | 74 | 9 | 0 |
| lib/clock | 3 | 66 | 0 | 2 | 1 | 1/16 | 3 | 62 | 0 | 0 |
| lib/repository | 4 | 110 | 1 | 1 | 2 | 9/17 | 3 | 56 | 0 | 0 |
| lib/outputfilter | 2 | 143 | 1 | 1 | 0 | 4/9 | 1 | 55 | 0 | 1 |
| lib/network | 3 | 38 | 0 | 1 | 2 | 0/5 | 2 | 34 | 4 | 0 |
| lib/standalone | 4 | 114 | 3 | 0 | 1 | 6/7 | 1 | 16 | 0 | 0 |
| build/docgen | 50 | 4788 | 0 | 0 | 50 | 0/362 | 0 | 0 | 4788 | 0 |
| lib/events | 15 | 764 | 15 | 0 | 0 | 114/123 | 0 | 0 | 0 | 0 |
| lib/jni | 1 | 109 | 0 | 0 | 1 | 0/8 | 0 | 0 | 109 | 0 |
| lib/platform | 14 | 490 | 0 | 2 | 12 | 1/76 | 0 | 0 | 490 | 0 |
| lib/testing | 3 | 130 | 0 | 0 | 3 | 0/22 | 0 | 0 | 130 | 0 |
| lib/unsafe | 2 | 100 | 0 | 1 | 1 | 3/9 | 0 | 0 | 100 | 0 |
| lib/versioning | 1 | 10 | 0 | 0 | 1 | 0/3 | 0 | 0 | 10 | 0 |
| lib/windows | 4 | 433 | 0 | 0 | 4 | 0/53 | 0 | 0 | 433 | 0 |
| net/starlark/annot | 6 | 587 | 0 | 0 | 6 | 0/48 | 0 | 0 | 587 | 0 |
| net/starlark/cmd | 1 | 176 | 0 | 0 | 1 | 0/4 | 0 | 0 | 176 | 0 |
| net/starlark/eval | 47 | 10600 | 3 | 30 | 14 | 103/923 | 0 | 0 | 10525 | 22 |
| net/starlark/lib | 1 | 668 | 1 | 0 | 0 | 17/26 | 0 | 0 | 0 | 0 |
| net/starlark/spelling | 1 | 56 | 1 | 0 | 0 | 3/3 | 0 | 0 | 0 | 0 |
| net/starlark/syntax | 50 | 6202 | 2 | 8 | 40 | 84/630 | 0 | 0 | 5959 | 0 |
| net/starlark/types | 2 | 425 | 0 | 0 | 2 | 0/72 | 0 | 0 | 425 | 0 |
<!-- END GENERATED PACKAGES -->


## Largest

<!-- BEGIN GENERATED LARGEST -->
| file | status | lines | marked | cluster | what it does (class comment) |
| --- | --- | --- | --- | --- | --- |
| lib/skyframe/SkyframeExecutor.java | PARTIAL | 3687 | 52/250 | incremental-evaluation | A helper object to support Skyframe-driven execution |
| lib/query2/SkyQueryEnvironment.java | PARTIAL | 1331 | 0/102 | query-output-formatters | AbstractBlazeQueryEnvironment that introspects the Skyframe graph to find forward and reverse edges |
| lib/buildtool/BuildTool.java | PARTIAL | 1546 | 13/56 | execution-and-action-graph | Provides the bulk of the implementation of the 'blaze build' command |
| lib/starlarkbuildapi/cpp/CcModuleApi.java | PARTIAL | 2054 | 14/32 | cpp-rules | (no class comment) |
| lib/runtime/BlazeRuntime.java | PARTIAL | 1534 | 25/82 | server-client-protocol | The BlazeRuntime class encapsulates the immutable configuration of the current instance |
| lib/runtime/UiStateTracker.java | PARTIAL | 1071 | 4/75 | runtime-and-ui | Tracks state for the UI |
| lib/skyframe/PackageFunction.java | PARTIAL | 1435 | 22/63 | packages-and-loading | A SkyFunction which computes a PackageValue if given a PackageIdentifier, or a PackagePieceValue.ForBuildFile if given a PackagePieceIdentifier.ForBuildFile |
| lib/profiler/Profiler.java | PARTIAL | 897 | 1/76 | profiler-and-metrics | Blaze internal profiler |
| lib/skyframe/SkyframeBuildView.java | PARTIAL | 1200 | 13/40 | analysis-core | Skyframe-based driver of analysis |
| lib/skyframe/SequencedSkyframeExecutor.java | PARTIAL | 851 | 4/53 | incremental-evaluation | A SkyframeExecutor that implicitly assumes that builds can be done incrementally from the most recent build |
| lib/skyframe/SkyframeActionExecutor.java | PARTIAL | 1551 | 44/89 | execution-and-action-graph | Action executor: takes care of preparing an action for execution, executing it, validating that all output artifacts were created, error reporting, etc |
| lib/analysis/starlark/StarlarkCustomCommandLine.java | PARTIAL | 1381 | 54/118 | analysis-core | Supports ctx.actions.args() from Starlark |
| lib/skyframe/rewinding/ActionRewindStrategy.java | PARTIAL | 781 | 2/36 | action-rewinding | Given an action that failed to execute because of lost inputs which were generated by other actions, finds the actions which generated them and the set of Skyframe nodes which must be rewound in order |
| lib/remote/RemoteExternalOverlayFileSystem.java | PARTIAL | 731 | 0/73 | remote-execution-and-caching | A file system that overlays the native file system with a RemoteExternalFileSystem for the "external" directory, which contains the contents of external repositories |
| lib/runtime/ExecutionGraphModule.java | PARTIAL | 778 | 3/48 | runtime-and-ui | Blaze module that writes a partial execution graph with performance data |
| lib/rules/java/JavaCompileAction.java | UNTOUCHED | 717 | 0/39 | java-rules-in-core | Action that represents a Java compilation |
| lib/remote/BazelOutputService.java | UNTOUCHED | 705 | 0/49 | remote-execution-and-caching | Output service implementation for the remote build with local output service daemon |
| lib/includescanning/IncludeParser.java | UNTOUCHED | 699 | 0/41 | include-scanning | Scans a source file and extracts the literal inclusions it specifies |
| lib/remote/http/HttpCacheClient.java | UNTOUCHED | 699 | 0/25 | remote-execution-and-caching | Implementation of RemoteCacheClient that can talk to a HTTP/1.1 backend |
| com/google/devtools/build/skyframe/AbstractParallelEvaluator.java | PARTIAL | 764 | 2/20 | execution-and-action-graph | Defines the evaluation action used in the multi-threaded Skyframe evaluation, and constructs the ParallelEvaluatorContext that the actions rely on |
| lib/actions/FileArtifactValue.java | PARTIAL | 995 | 57/178 | execution-and-action-graph | A value that represents a file for the purposes of up-to-dateness checks of actions |
| lib/buildeventservice/BuildEventServiceModule.java | PARTIAL | 857 | 9/42 | build-event-service | Module responsible for the Build Event Transport (BEP) and Build Event Service (BES) functionality |
| lib/skyframe/serialization/analysis/FileDependencySerializer.java | UNTOUCHED | 669 | 0/39 | remote-analysis-caching-serialization | Records FileKey, DirectoryListingKey or AbstractNestedFileOpNodes invalidation to a remote KeyValueWriter |
| lib/buildeventservice/BuildEventServiceUploader.java | UNTOUCHED | 661 | 0/36 | build-event-service | Uploader of Build Events to the Build Event Service (BES) |
| lib/includescanning/LegacyIncludeScanner.java | UNTOUCHED | 657 | 0/29 | include-scanning | C include scanner |
| lib/skyframe/FilesystemValueChecker.java | UNTOUCHED | 653 | 0/29 | incremental-evaluation | A helper class to find dirty values by accessing the filesystem directly (contrast with DiffAwareness) |
| com/google/devtools/common/options/InvocationPolicyEnforcer.java | PARTIAL | 666 | 2/24 | base-libraries | Enforces the FlagPolicys (from an InvocationPolicy proto) on an OptionsParser by validating and changing the flag values in the given OptionsParser |
| lib/collect/compacthashmap/CompactHashMap.java | UNTOUCHED | 591 | 0/76 | base-libraries | CompactHashMap is an implementation of a Map |
| lib/runtime/UiEventHandler.java | PARTIAL | 946 | 24/64 | runtime-and-ui | Presents events to the user in the terminal |
| com/google/devtools/build/skyframe/SkyFunctionEnvironment.java | PARTIAL | 944 | 24/60 | execution-and-action-graph | (no class comment) |
| lib/analysis/BuildView.java | PARTIAL | 821 | 8/25 | analysis-core | The BuildView presents a semantically-consistent and transitively-closed dependency graph for some set of packages |
| lib/bazel/rules/BazelRulesModule.java | PARTIAL | 835 | 1/3 | python-and-other-native-rules | Module implementing the rule set of Bazel |
| lib/runtime/commands/DumpCommand.java | PARTIAL | 613 | 2/18 | commands | Implementation of the dump command |
| lib/profiler/CollectLocalResourceUsage.java | UNTOUCHED | 539 | 0/30 | profiler-and-metrics | Thread to collect local resource usage data and log into JSON profile |
| lib/starlarkbuildapi/java/JavaCommonApi.java | PARTIAL | 664 | 4/21 | java-rules-in-core | (no class comment) |
| lib/skyframe/serialization/analysis/FileDependencyDeserializer.java | UNTOUCHED | 530 | 0/41 | remote-analysis-caching-serialization | Deserializes dependency information persisted by FileDependencySerializer |
| lib/server/GrpcServerImpl.java | PARTIAL | 568 | 2/29 | server-client-protocol | gRPC server class |
| lib/skyframe/config/FlagSetFunction.java | UNTOUCHED | 523 | 0/14 | configuration-and-transitions | A SkyFunction that, given an scl file path and the name of scl configs, does the following: calls com.google.devtools.build.lib.skyframe.ProjectFunction to load the content of scl files given the prov |
| lib/skyframe/packages/AbstractPackageLoader.java | UNTOUCHED | 523 | 0/35 | execution-and-action-graph | Abstract base class of a PackageLoader implementation that has no incrementality or caching |
| lib/rules/java/JavaHeaderCompileAction.java | UNTOUCHED | 512 | 0/36 | java-rules-in-core | Action for Java header compilation, to be used if --java_header_compilation is enabled |
| lib/actions/SpawnResult.java | PARTIAL | 537 | 6/93 | sandboxing-and-local-exec | The result of a Spawn's execution |
| lib/starlarkdocextract/ModuleInfoExtractor.java | UNTOUCHED | 499 | 0/35 | starlark-debugger-and-docs | API documentation extractor for a compiled, loaded Starlark module |
| lib/dynamic/DynamicSpawnStrategy.java | PARTIAL | 546 | 2/20 | dynamic-execution | A spawn strategy that speeds up incremental builds while not slowing down full builds |
| lib/runtime/CommandEnvironment.java | PARTIAL | 820 | 37/91 | runtime-and-ui | Encapsulates the state needed for a single command |
| lib/rules/java/JavaCompilationHelper.java | UNTOUCHED | 478 | 0/26 | java-rules-in-core | A helper class for compiling Java targets |
| lib/remote/RemoteRepoContentsCacheImpl.java | UNTOUCHED | 475 | 0/19 | remote-execution-and-caching | A cache for the contents of external repositories that is backed by an ordinary remote cache |
| lib/skyframe/serialization/SharedValueDeserializationContext.java | UNTOUCHED | 471 | 0/34 | remote-analysis-caching-serialization | Implementation that supports sharing of sub-objects between objects |
| lib/worker/WorkerSpawnRunner.java | PARTIAL | 578 | 4/20 | workers | A spawn runner that launches Spawns the first time they are used in a persistent mode and then shards work over all the processes |
| lib/worker/WorkRequestHandler.java | UNTOUCHED | 457 | 0/40 | workers | A helper class that handles WorkRequests (https://bazel.build/docs/persistent-workers), including multiplex workers (https://bazel.build/docs/multiplex-worker) |
| lib/actions/ArtifactCodecs.java | UNTOUCHED | 451 | 0/55 | execution-and-action-graph | Codec implementations for Artifact subclasses |
| lib/rules/genquery/GenQuery.java | UNTOUCHED | 449 | 0/14 | query-output-formatters | An implementation of the 'genquery' rule |
| lib/runtime/BlazeCommandDispatcher.java | PARTIAL | 895 | 13/26 | server-client-protocol | Dispatches to the Blaze commands; that is, given a command line, this abstraction looks up the appropriate command object, parses the options required by the object, and calls its exec method |
| lib/skyframe/ProjectFunction.java | UNTOUCHED | 437 | 0/20 | execution-and-action-graph | A SkyFunction that loads metadata from a PROJECT.scl file |
| lib/graph/Digraph.java | UNTOUCHED | 431 | 0/43 | base-libraries | Digraph a generic directed graph or "digraph", suitable for modeling asymmetric binary relations |
| lib/actions/SpawnMetrics.java | PARTIAL | 458 | 5/80 | sandboxing-and-local-exec | Timing, size, and memory statistics for a Spawn execution |
| com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java | UNTOUCHED | 412 | 0/21 | incremental-evaluation | A visitor that is useful for invalidating transitive dependencies of Skyframe nodes |
| lib/rules/java/JavaCompileActionBuilder.java | UNTOUCHED | 411 | 0/36 | java-rules-in-core | Java compilation action builder |
| lib/packages/PackagePiece.java | UNTOUCHED | 409 | 0/44 | symbolic-macros-and-lazy-loading | (no class comment) |
| lib/packages/AutoloadSymbols.java | PARTIAL | 680 | 10/25 | packages-and-loading | Implementation of --incompatible_autoload_externally |
| lib/skyframe/DependencyResolver.java | PARTIAL | 675 | 12/30 | execution-and-action-graph | Helper logic for ConfiguredTargetFunction and AspectFunction: performs the analysis phase through computation of prerequisites |
<!-- END GENERATED LARGEST -->


## Clusters

<!-- BEGIN GENERATED CLUSTERS -->
| cluster | NEEDED files not ported | of them UNTOUCHED | lines | unported lines (est.) | unmarked methods | unported! sites | depends on (import edges into other clusters' unported files) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| execution-and-action-graph | 204 | 106 | 26291 | 21005 | 2024 | 34 | incremental-evaluation (126), base-libraries (114), analysis-core (58), remote-analysis-caching-serialization (46), packages-and-loading (39), configuration-and-transitions (17) |
| base-libraries | 156 | 103 | 15129 | 13438 | 1591 | 6 | remote-analysis-caching-serialization (40), execution-and-action-graph (14), incremental-evaluation (4), profiler-and-metrics (2), java-rules-in-core (1), runtime-and-ui (1) |
| remote-analysis-caching-serialization | 117 | 116 | 11211 | 11211 | 1143 | 0 | execution-and-action-graph (36), base-libraries (23), packages-and-loading (16), incremental-evaluation (11), analysis-core (6), configuration-and-transitions (3) |
| incremental-evaluation | 52 | 33 | 11466 | 10169 | 983 | 5 | execution-and-action-graph (230), base-libraries (60), packages-and-loading (33), configuration-and-transitions (15), analysis-core (13), profiler-and-metrics (11) |
| packages-and-loading | 97 | 35 | 11388 | 8561 | 811 | 13 | execution-and-action-graph (169), base-libraries (53), symbolic-macros-and-lazy-loading (18), remote-analysis-caching-serialization (12), incremental-evaluation (9), repositories-and-bzlmod (5) |
| analysis-core | 87 | 37 | 11240 | 8203 | 676 | 17 | execution-and-action-graph (84), base-libraries (49), packages-and-loading (34), configuration-and-transitions (26), remote-analysis-caching-serialization (16), incremental-evaluation (11) |
| remote-execution-and-caching | 61 | 37 | 8186 | 7527 | 565 | 7 | execution-and-action-graph (22), base-libraries (15), incremental-evaluation (9), repositories-and-bzlmod (7), sandboxing-and-local-exec (5), profiler-and-metrics (4) |
| query-output-formatters | 48 | 29 | 6913 | 6363 | 416 | 4 | execution-and-action-graph (40), base-libraries (37), packages-and-loading (22), analysis-core (15), incremental-evaluation (13), configuration-and-transitions (6) |
| java-rules-in-core | 35 | 28 | 6245 | 5884 | 522 | 1 | base-libraries (40), packages-and-loading (29), execution-and-action-graph (17), analysis-core (9), sandboxing-and-local-exec (5), cpp-rules (3) |
| runtime-and-ui | 42 | 25 | 6438 | 5332 | 522 | 8 | execution-and-action-graph (44), base-libraries (38), build-event-service (15), server-client-protocol (15), analysis-core (11), incremental-evaluation (9) |
| configuration-and-transitions | 66 | 14 | 6569 | 4950 | 492 | 14 | execution-and-action-graph (71), analysis-core (42), base-libraries (15), incremental-evaluation (14), packages-and-loading (4), python-and-other-native-rules (2) |
| workers | 28 | 22 | 4650 | 4445 | 346 | 2 | sandboxing-and-local-exec (26), execution-and-action-graph (13), profiler-and-metrics (5), base-libraries (4), runtime-and-ui (3), build-event-service (1) |
| server-client-protocol | 28 | 15 | 5576 | 4229 | 267 | 4 | base-libraries (43), runtime-and-ui (29), profiler-and-metrics (7), build-event-service (6), incremental-evaluation (6), sandboxing-and-local-exec (5) |
| profiler-and-metrics | 35 | 29 | 4302 | 4155 | 358 | 2 | base-libraries (24), execution-and-action-graph (15), sandboxing-and-local-exec (8), runtime-and-ui (6), build-event-service (3), incremental-evaluation (3) |
| cpp-rules | 29 | 21 | 5444 | 4137 | 291 | 0 | base-libraries (17), packages-and-loading (16), execution-and-action-graph (11), analysis-core (6), remote-analysis-caching-serialization (4), configuration-and-transitions (2) |
| sandboxing-and-local-exec | 49 | 26 | 4648 | 4110 | 518 | 6 | execution-and-action-graph (20), base-libraries (11), runtime-and-ui (6), include-scanning (1), profiler-and-metrics (1), server-client-protocol (1) |
| repositories-and-bzlmod | 60 | 11 | 3916 | 2940 | 251 | 3 | execution-and-action-graph (67), base-libraries (18), incremental-evaluation (5), profiler-and-metrics (4), packages-and-loading (2), runtime-and-ui (2) |
| include-scanning | 13 | 12 | 2754 | 2738 | 177 | 0 | execution-and-action-graph (31), base-libraries (8), sandboxing-and-local-exec (5), packages-and-loading (4), incremental-evaluation (3), profiler-and-metrics (3) |
| starlark-debugger-and-docs | 20 | 17 | 2736 | 2680 | 181 | 2 | packages-and-loading (11), base-libraries (8), runtime-and-ui (4), analysis-core (2), execution-and-action-graph (2), repositories-and-bzlmod (2) |
| build-event-service | 27 | 20 | 2917 | 2643 | 216 | 3 | base-libraries (11), server-client-protocol (7), runtime-and-ui (5), profiler-and-metrics (3), execution-and-action-graph (2), analysis-core (1) |
| commands | 16 | 2 | 2373 | 2092 | 94 | 0 | runtime-and-ui (24), execution-and-action-graph (13), base-libraries (12), incremental-evaluation (11), analysis-core (8), configuration-and-transitions (8) |
| symbolic-macros-and-lazy-loading | 16 | 9 | 2117 | 1890 | 161 | 0 | execution-and-action-graph (43), packages-and-loading (20), base-libraries (12), incremental-evaluation (6) |
| action-rewinding | 9 | 6 | 1309 | 1246 | 92 | 1 | execution-and-action-graph (24), base-libraries (4), runtime-and-ui (3), analysis-core (1), profiler-and-metrics (1), remote-analysis-caching-serialization (1) |
| dynamic-execution | 7 | 6 | 1059 | 1004 | 54 | 2 | sandboxing-and-local-exec (10), execution-and-action-graph (7), runtime-and-ui (3), profiler-and-metrics (2), base-libraries (1) |
| python-and-other-native-rules | 12 | 3 | 1342 | 974 | 28 | 2 | analysis-core (19), base-libraries (3), execution-and-action-graph (3), cpp-rules (2), packages-and-loading (2), runtime-and-ui (2) |
| coverage | 11 | 5 | 857 | 769 | 49 | 5 | execution-and-action-graph (15), analysis-core (5), base-libraries (5), runtime-and-ui (3), sandboxing-and-local-exec (3), action-rewinding (2) |
| android-apple-objc-proto | 8 | 4 | 596 | 534 | 36 | 0 | base-libraries (5), analysis-core (4), packages-and-loading (3), python-and-other-native-rules (2) |
| native-aspects | 2 | 0 | 165 | 147 | 8 | 3 | remote-analysis-caching-serialization (2) |
<!-- END GENERATED CLUSTERS -->


## Tests

<!-- BEGIN GENERATED TESTS -->
Java test classes `src/test/java/.../FooTest.java` whose production class `Foo` is PORTED or PARTIAL: 490 (of 1171 `*Test.java` files in the pinned source tree).

|  | count |
| --- | --- |
| @Test methods in those classes | 6331 |
| with a Rust #[test] of the snake_case name in a file named after the class | 3662 |
| with a Rust #[test] of that name anywhere in crates/ | 4243 |
| of the matched, every matching Rust test is #[ignore] | 709 |
| Rust #[test] functions in crates/ (all) | 8340 |
| Rust #[test] functions marked #[ignore] (all) | 1729 |

Top 30 `#[ignore]` reasons among the matched tests:

| reason (first 110 chars) | tests |
| --- | --- |
| incremental: needs invalidation of changed nodes (dirty nodes, versions, change \ pruning) | 58 |
| symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's Pac | 31 |
| needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment | 19 |
| needs BuildIntegrationTestCase (a whole build through the runtime) | 16 |
| needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) | 15 |
| SkyFunction.Reset and node rewinding are not ported | 14 |
| needs the mock C++ toolchains (cc_binary from the mock rules_cc) | 14 |
| needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports | 12 |
| needs a TestRunnerAction test double (Java mocks it) | 12 |
| needs deserialization futures (NestedSet.withFuture); serialization skipped | 12 |
| needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class pro | 11 |
| not ported yet: instantiates a RuleClass built in the test with RuleClass.createRule on a Package.Builder (Rul | 10 |
| not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals | 9 |
| needs a Skyframe evaluator with the file and directory listing functions over a scratch file system | 8 |
| needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP) | 8 |
| needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture | 8 |
| MemoizingEvaluator.delete / deleteDirty are not ported | 7 |
| needs idle tasks | 7 |
| needs the ConfiguredTargetQueryHelper analysis harness | 7 |
| BuildInfoFileWriteAction.execute is in blaze_exec \ (src/analysis/actions/build_info_file_write_action.rs), wh | 6 |
| EvaluationProgressReceiver is not ported | 6 |
| events stored on nodes and replayed (EventFilter) are not ported: one build per \ process | 6 |
| needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) and TestConstants.P | 6 |
| needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock), scratch BUILD/bzl  | 6 |
| needs the BuildViewTestCase harness (blaze_sky_top::testing) | 6 |
| Environment.dependOnFuture (external deps) is not ported | 5 |
| WindowsFormatter: PORT skipped, Windows only | 5 |
| execution is in blaze_exec, which depends on this crate; ported there: \ tests/analysis_actions_execute_test.r | 5 |
| needs TestAction and ActionsTestUtil.createArtifact (actions test utilities) | 5 |
| needs a Bzlmod + BZL_LOAD fixture with a fake registry | 5 |

Top 30 `#[ignore]` reasons over all Rust tests in crates/:

| reason (first 110 chars) | tests |
| --- | --- |
| needs package loading and target pattern evaluation from blaze_sky_loading (AbstractQueryHelper/SkyframeQueryH | 181 |
| needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching) | 84 |
| needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descrip | 81 |
| needs InvocationPolicyEnforcer (not ported) | 65 |
| incremental: needs invalidation of changed nodes (dirty nodes, versions, change \ pruning) | 58 |
| needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander | 33 |
| needs BuildIntegrationTestCase (in-process build driver) | 31 |
| symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's Pac | 31 |
| needs the mock C++/Java/Python toolchains (java_library from the mock rules_java) | 30 |
| needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc) | 28 |
| needs analysis (getConfiguredTarget and the providers or actions of the configured target) | 24 |
| symbolic macros are not ported (MacroClass.instantiateAndAddMacro) | 22 |
| needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment | 19 |
| needs analysis (createRuleContext: the StarlarkRuleContext of a configured target) | 18 |
| inspects the StarlarkDefinedAspect or the attribute aspects behind ev.lookup; portable with the .bzl harness o | 16 |
| needs BuildIntegrationTestCase (a whole build through the runtime) | 16 |
| inspects the RuleClass behind ev.lookup (getRuleClass); portable with the .bzl harness of starlark_defs/tests. | 15 |
| needs ConfigurationTestCase (create/createExec through SkyframeExecutor with AnalysisMock) | 15 |
| SkyFunction.Reset and node rewinding are not ported | 14 |
| inspects Java values behind ev.lookup (StarlarkProvider key and schema, StructImpl provider and field names, D | 14 |
| needs the mock C++ toolchains (cc_binary from the mock rules_cc) | 14 |
| needs deserialization futures (NestedSet.withFuture); serialization skipped | 13 |
| the rule comes from a BUILD file that loads @rules_cc//cc:cc_binary.bzl (Java scratchRule): needs blaze_sky_lo | 13 |
| inspects the built Attribute (buildAttribute); portable with the .bzl harness of starlark_defs/tests.rs (build | 12 |
| needs TreeArtifactValue.Builder (TreeArtifactFactory) and visitTree ports | 12 |
| needs a TestRunnerAction test double (Java mocks it) | 12 |
| needs the test rule feature_flag_setter (FeatureFlagSetterRule, registered by the test's custom rule class pro | 11 |
| needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule) | 10 |
| needs ShellQuotedParamsFilePreProcessor (not ported) | 10 |
| needs the BuildViewTestCase harness (blaze_sky_top::testing) | 10 |
<!-- END GENERATED TESTS -->


## Cpp

<!-- BEGIN GENERATED CPP -->
C++ client (`src/main/cpp`): 59 files, 10131 lines; Linux-relevant 52 files, 7445 lines; named by a marker: 42 files (7208 lines).

| file | lines | markers | note |
| --- | --- | --- | --- |
| archive_utils.cc | 237 | 7 |  |
| archive_utils.h | 53 | 3 |  |
| bazel_startup_options.cc | 82 | 7 |  |
| bazel_startup_options.h | 26 | 2 |  |
| blaze.cc | 1504 | 45 |  |
| blaze.h | 17 | 0 |  |
| blaze_util.cc | 217 | 14 |  |
| blaze_util.h | 70 | 6 |  |
| blaze_util_bsd.cc | 188 | 0 | OS-only |
| blaze_util_darwin.cc | 238 | 0 | OS-only |
| blaze_util_linux.cc | 171 | 10 |  |
| blaze_util_platform.h | 128 | 2 |  |
| blaze_util_posix.cc | 669 | 41 |  |
| blaze_util_windows.cc | 1096 | 0 | OS-only |
| command_extension_adder.h | 13 | 0 |  |
| get_self_path_linux.cc | 24 | 1 |  |
| main.cc | 36 | 3 |  |
| option_processor-internal.h | 26 | 0 |  |
| option_processor.cc | 560 | 28 |  |
| option_processor.h | 91 | 3 |  |
| option_processor_unix.cc | 30 | 1 |  |
| option_processor_windows.cc | 65 | 0 | OS-only |
| rc_file.cc | 261 | 8 |  |
| rc_file.h | 64 | 3 |  |
| sem_ver.cc | 107 | 6 |  |
| sem_ver.h | 36 | 1 |  |
| server_process_info.cc | 17 | 2 |  |
| server_process_info.h | 18 | 1 |  |
| startup_interceptor.h | 14 | 0 |  |
| startup_options.cc | 589 | 35 |  |
| startup_options.h | 147 | 3 |  |
| util/bazel_log_handler.cc | 123 | 9 |  |
| util/bazel_log_handler.h | 31 | 1 |  |
| util/errors.h | 8 | 0 |  |
| util/errors_posix.cc | 15 | 2 |  |
| util/errors_windows.cc | 26 | 0 | OS-only |
| util/exit_code.h | 14 | 2 |  |
| util/file.cc | 87 | 1 |  |
| util/file.h | 26 | 0 |  |
| util/file_platform.h | 98 | 0 |  |
| util/file_posix.cc | 437 | 48 |  |
| util/file_windows.cc | 660 | 0 | OS-only |
| util/logging.cc | 76 | 6 |  |
| util/logging.h | 123 | 3 |  |
| util/md5.cc | 244 | 1 |  |
| util/md5.h | 33 | 4 |  |
| util/numbers.cc | 150 | 6 |  |
| util/numbers.h | 9 | 0 |  |
| util/path.cc | 25 | 4 |  |
| util/path.h | 10 | 0 |  |
| util/path_platform.h | 72 | 3 |  |
| util/path_posix.cc | 121 | 27 |  |
| util/path_windows.cc | 413 | 0 | OS-only |
| util/port.cc | 16 | 0 |  |
| util/port.h | 59 | 1 |  |
| util/strings.cc | 317 | 6 |  |
| util/strings.h | 67 | 1 |  |
| workspace_layout.cc | 54 | 7 |  |
| workspace_layout.h | 23 | 3 |  |

Protos (`src/main/protobuf`): 31 files; compiled by `blaze_protos/build.rs`: 18.

| proto | lines | compiled |
| --- | --- | --- |
| action_cache.proto | 27 | yes |
| analysis_v2.proto | 104 | yes |
| bazel_flags.proto | 25 | yes |
| bazel_output_service.proto | 91 |  |
| bazel_output_service_rev2.proto | 18 |  |
| build.proto | 247 | yes |
| builtin.proto | 38 |  |
| cache_salt.proto | 12 | yes |
| command_line.proto | 29 | yes |
| command_server.proto | 107 | yes |
| crash_debugging.proto | 15 |  |
| crosstool_config.proto | 166 | yes |
| deps.proto | 26 |  |
| desugar_deps.proto | 24 |  |
| execution_graph.proto | 33 |  |
| execution_statistics.proto | 28 |  |
| extra_actions_base.proto | 90 |  |
| failure_details.proto | 1078 | yes |
| file_invalidation_data.proto | 25 |  |
| invocation_policy.proto | 49 | yes |
| java_compilation.proto | 12 |  |
| memory_pressure.proto | 15 |  |
| option_filters.proto | 34 | yes |
| remote_execution_log.proto | 81 | yes |
| remote_scrubbing.proto | 25 | yes |
| spawn.proto | 155 | yes |
| stardoc_output.proto | 139 | yes |
| strategy_policy.proto | 17 | yes |
| test_status.proto | 65 | yes |
| worker_protocol.proto | 21 | yes |
| xcode_config.proto | 23 |  |
<!-- END GENERATED CPP -->


## Rules

<!-- BEGIN GENERATED RULES -->
| reason | path regex (lib/ = com/google/devtools/build/lib/) | why |
| --- | --- | --- |
| OUT_OF_SCOPE_OS | `/windows/` | Windows-only package |
| OUT_OF_SCOPE_OS | `/(Windows\|Win32)[A-Z]\w*\.java$\|Windows\w*\.java$` | Windows class |
| OUT_OF_SCOPE_OS | `/(Darwin\|MacOS\|Macos\|Xcode\|XCode)\w*\.java$` | macOS class |
| OUT_OF_SCOPE_OS | `lib/platform/` | system monitors whose native side is a no-op on Linux (src/main/native/unix_jni_linux.cc:78-130) |
| OUT_OF_SCOPE_OS | `lib/sandbox/WindowsSandbox\|lib/sandbox/DarwinSandbox` | non-Linux sandbox |
| JVM_ONLY | `/processor/` | javac annotation processor (build-time Java tooling) |
| JVM_ONLY | `lib/skyframe/serialization/autocodec/` | AutoCodec annotations and processor |
| JVM_ONLY | `lib/unsafe/` | sun.misc.Unsafe access |
| JVM_ONLY | `lib/jni/` | JNI library loading |
| JVM_ONLY | `net/starlark/java/annot/` | Java annotations for the Starlark interpreter |
| JVM_ONLY | `lib/util/(StringUnsafe\|HeapOffsetHelper\|ObjectSizer)\w*\.java$` | JVM object layout |
| JVM_ONLY | `lib/runtime/(MemoryPressure\w*\|GcThrashingDetector\|GcChurningDetector\|RetainedHeapLimiter\|HeapDumpUtil\|ThreadDumper)\.java$` | JVM heap/GC monitoring |
| JVM_ONLY | `lib/(util\|runtime)/(ThreadDump\w*\|GCStatsRecorder)\.java$` | JVM thread dumps and GC statistics |
| JVM_ONLY | `lib/server/GcAndInternerShrinkingIdleTask\.java$` | JVM GC and interner shrinking |
| JVM_ONLY | `lib/skyframe/serialization/strings/UnsafeStringCodec\.java$` | sun.misc.Unsafe string codec |
| JVM_ONLY | `lib/concurrent/(ForkJoinQuiescingExecutor\|NamedForkJoinPool)\.java$` | java.util.concurrent ForkJoin plumbing |
| OUT_OF_SCOPE_BY_CONTRACT | `com/google/devtools/build/docgen/` | doc generator, not part of the bazel binary |
| OUT_OF_SCOPE_BY_CONTRACT | `lib/profiler/memory/` | memory instrumentation (PORTING.md Skip only) |
| OUT_OF_SCOPE_BY_CONTRACT | `lib/(profiler/MemoryProfiler\|buildtool/SkyframeMemoryDumper)\.java$` | memory instrumentation (--memory_profile, dump --memory) |
| OUT_OF_SCOPE_BY_CONTRACT | `lib/metrics/(PostGCMemoryUseRecorder\|MemoryUseRecorder\|GarbageCollectionMetricsUtils)\w*\.java$` | memory instrumentation |
| OUT_OF_SCOPE_BY_CONTRACT | `/[Gg]oogle(?!Auth)\w*\.java$` | Google-internal (name says Google; GoogleAuth* kept) |
| TEST_SUPPORT | `/(testing\|testutil\|testutils)/` | test-support package |
| TEST_SUPPORT | `\w+ForTesting\.java$\|/Testing\w*\.java$` | test-support class |
| STARLARK_RUST_DECISION | `^net/starlark/java/` | replaced by starlark-rust (decisions.md 2026-11-17) |

Then: `package-info.java`, or no method with a body and at most 30 lines of code: GENERATED_OR_TRIVIAL. Everything else: NEEDED.

Clusters (first match wins):

| cluster | patterns |
| --- | --- |
| server-client-protocol | `lib/server/`<br>`lib/runtime/(BlazeCommandDispatcher\|CommandDispatcher\|BlazeServerStartupOptions\|ServerCapabilities\|BlockWaitingModule\|IdleTask\|InstrumentationOutput\w*\|CommandLinePathFactory\|CommandLog\|ClientOptions\|ConfigFlagDefinitions\|UiOptions\|LockingMode\|RemoteServer\w*\|GrpcServer\w*)`<br>`lib/runtime/(BlazeRuntime\|BlazeWorkspace\|WorkspaceBuilder)\.java$`<br>`lib/runtime/commands/(Shutdown\|Clean)Command`<br>`lib/util/(ProcessUtils\|ThreadUtils\|DebugLoggerConfigurator\|LoggingUtil\|InterruptedFailureDetails)`<br>`lib/platform/` |
| incremental-evaluation | `com/google/devtools/build/skyframe/(InMemoryGraph\|InMemoryNodeEntry\|IncrementalInMemoryNodeEntry\|NodeEntry\|DirtyBuildingState\|InvalidatingNodeVisitor\|DirtyTrackingProgressReceiver\|Differencer\|RecordingDifferencer\|SequencedRecordingDifferencer\|Version\|IntVersion\|Delta\|EvaluationVersion\|ThinNodeEntry\|DeletingInvalidationState\|DirtinessCheckingState\|ChangePruning\|DelegatingNodeEntry\|EmptyNodeEntry\|NodeBatch\|QueryableGraph\|ProcessableGraph\|NotifyingHelper\|InMemoryMemoizingEvaluator\|AbstractInMemoryMemoizingEvaluator\|MemoizingEvaluator\|SkyValueDirtinessChecker\|GroupedDeps\|InvalidationState\|ReverseDepsUtility\|DirtyType\|LifecycleState\|EvaluationContext\|IncrementalityIntent\|InMemoryGraphImpl\|AbstractIncrementalInMemoryMemoizingEvaluator\|EdgeType\|PartialReevaluationMailbox)`<br>`lib/skyframe/(DiffAwareness\w*\|LocalDiffAwareness\|FileSystemValueChecker\w*\|FilesystemValueChecker\w*\|DirtinessChecker\w*\|ExternalFilesHelper\|SkyValueDirtinessChecker\|SequencedSkyframeExecutor\w*\|SkyframeIncrementalBuildMonitor\|PackageRootsNoSymlinkCreation\|DiffAwarenessManager\|ExternalDirtinessChecker\|ModifiedOutputsReceiver\|TopDownActionCache\|BasicFilesystemDirtinessChecker\|MissingDiffDirtinessChecker\|UnionDirtinessChecker\|DirectoryListingStateValue\|PrecomputedValue\|ActionExecutionInactivityWatchdog)`<br>`com/google/devtools/build/skyframe/state/`<br>`lib/skyframe/(SkyframeExecutor\|SkyframeExecutorWrappingWalkableGraph\|SkyFocus\w*)\.java$`<br>`lib/skyframe/skyfocus/`<br>`lib/vfs/(BatchStat\|ModifiedFileSet\|DigestUtils\|SyscallCache\|BulkDeleter\|OutputService\|DelegateFileSystem\|FileStateKey)` |
| symbolic-macros-and-lazy-loading | `/(MacroClass\|MacroInstance\|MacroFrame\|MacroInstanceFunction\|MacroInstanceValue\|EvalMacroFunction\|MacroFunctionApi\|StarlarkMacro\w*\|LazyMacro\w*\|PackagePiece\w*\|PackagePieceIdentifier\|Materializer\w*\|TargetRecorder\|MacroVisibility\w*\|Subrule\w*\|StarlarkSubrule\w*)\.java$`<br>`lib/packages/producers/`<br>`lib/skyframe/(PackagePiece\|EvalMacro\|MacroInstance\|PackageFunctionWithMultipleGlobDeps\|PackageFunctionWithSingleGlobDep\|PackageFunctionWithoutGlobDeps\|GlobsFunction\|GlobsValue)` |
| native-aspects | `lib/analysis/(NativeAspectClass\|ConfiguredAspectFactory\|AspectCollection\|AspectResolver\|DependencyResolutionHelpers)`<br>`lib/packages/(NativeAspectClass\|AspectParameters\|AspectDescriptor)`<br>`lib/query2/query/aspectresolvers/` |
| coverage | `/(Coverage\w*\|InstrumentedFiles\w*\|LcovMerger\w*\|BaselineCoverage\w*\|InstrumentationFilter\w*\|CoverageReport\w*)\.java$`<br>`lib/bazel/coverage/`<br>`lib/runtime/commands/CoverageCommand` |
| workers | `lib/worker/`<br>`lib/sandbox/\w*Worker` |
| dynamic-execution | `lib/dynamic/` |
| action-rewinding | `lib/skyframe/rewinding/`<br>`/(LostInputs\w*\|ActionRewind\w*\|RewindingStrategy\w*\|LostInputsExecException\|ImportantOutputHandler\w*)\.java$` |
| include-scanning | `lib/includescanning/`<br>`lib/rules/cpp/(IncludeScann\w*\|IncludeParser\|IncludeProblems\|SwigIncludeScanner\|CppIncludeScanningContext\w*\|ScanningIncludeHelper)` |
| java-rules-in-core | `lib/rules/java/`<br>`lib/bazel/rules/java/`<br>`lib/starlarkbuildapi/java/`<br>`/(Java\w*\|Jvm\w*)\.java$` |
| android-apple-objc-proto | `lib/rules/(android\|apple\|objc\|proto)/`<br>`lib/starlarkbuildapi/(android\|apple\|objc\|proto)/`<br>`/(Android\|Apple\|Objc\|J2Objc\|XcodeConfig)\w*\.java$` |
| query-output-formatters | `lib/query2/query/output/`<br>`lib/query2/(aquery\|cquery\|common)/`<br>`lib/query2/\w+\.java$`<br>`lib/query2/compat/`<br>`lib/rules/genquery/`<br>`lib/runtime/commands/(Query\|CQuery\|AQuery)\w*`<br>`lib/buildtool/(QueryProcessor\|AqueryProcessor\|CqueryProcessor\|PostAnalysisQueryProcessor)`<br>`lib/query2/engine/`<br>`lib/query2/query/` |
| profiler-and-metrics | `lib/profiler/`<br>`lib/metrics/`<br>`lib/runtime/(MemoryProfiler\|BuildSummaryStatsModule\|CriticalPath\w*\|CommandProfilerModule\|ProfilerStartedEvent\|BugReportingExceptionHandler\|ActionExecutionStatusReporter)`<br>`lib/packages/metrics/`<br>`lib/buildtool/buildevent/BuildMetrics` |
| remote-execution-and-caching | `lib/remote/`<br>`lib/authandtls/`<br>`lib/runtime/(CommandLinePathFactory\|RemoteRepository\w*)`<br>`lib/bazel/repository/downloader/(GrpcRemoteDownloader\|Http\w*\|DelegatingDownloader)`<br>`lib/exec/(SpawnCache\|RemoteLocalFallbackRegistry)` |
| build-event-service | `lib/buildeventservice/`<br>`lib/buildeventstream/`<br>`lib/runtime/(BuildEvent\w*\|BuildEventStreamer\w*\|BuildEventArtifactUploader\w*\|CountingArtifactGroupNamer\|SynchronizedOutputStream)`<br>`lib/buildtool/buildevent/` |
| remote-analysis-caching-serialization | `lib/skyframe/serialization/` |
| starlark-debugger-and-docs | `lib/starlarkdebug/`<br>`lib/starlarkdocextract/`<br>`lib/rules/starlarkdocextract/`<br>`lib/bazel/debug/` |
| commands | `lib/runtime/commands/`<br>`lib/bazel/commands/`<br>`lib/runtime/mobileinstall/`<br>`lib/bazel/bzlmod/modcommand/`<br>`lib/runtime/(ProjectFile\|Project\w*)` |
| sandboxing-and-local-exec | `lib/sandbox/`<br>`lib/exec/`<br>`lib/shell/`<br>`lib/standalone/`<br>`lib/unix/`<br>`lib/actions/(Spawn\|Resource\|Local)\w*` |
| repositories-and-bzlmod | `lib/bazel/bzlmod/`<br>`lib/bazel/repository/`<br>`lib/rules/repository/`<br>`lib/repository/`<br>`lib/skyframe/(Repository\w*\|\w*RepositoryDirectory\w*\|ClientEnvironment\w*\|RepoFile\w*\|Bzlmod\w*)`<br>`lib/bazel/(BazelRepositoryModule\|BazelRepositoryModule\w*)` |
| configuration-and-transitions | `lib/analysis/config/`<br>`lib/skyframe/config/`<br>`lib/rules/config/`<br>`lib/analysis/platform/`<br>`lib/rules/platform/`<br>`lib/skyframe/toolchains/`<br>`lib/analysis/constraints/`<br>`lib/analysis/producers/` |
| cpp-rules | `lib/rules/cpp/`<br>`lib/bazel/rules/cpp/`<br>`lib/starlarkbuildapi/cpp/` |
| python-and-other-native-rules | `lib/rules/`<br>`lib/bazel/rules/` |
| analysis-core | `lib/analysis/`<br>`lib/starlarkbuildapi/`<br>`lib/skyframe/(ConfiguredTarget\w*\|Aspect\w*\|\w*Analysis\w*\|SkyframeBuildView\|BuildConfiguration\w*\|StarlarkAspect\w*\|TargetCompletor\|ConfiguredValueCreation\w*\|PlatformMapping\w*)` |
| packages-and-loading | `lib/packages/`<br>`lib/pkgcache/`<br>`lib/skyframe/(Package\w*\|Bzl\w*\|Glob\w*\|TargetPattern\w*\|\w*Recursive\w*\|StarlarkBuiltins\w*\|Ignored\w*\|ContainingPackage\w*\|CollectPackages\w*\|PrepareDepsOfPattern\w*\|BuildFile\w*\|ProcessPackageDirectory\w*\|TargetMarker\w*\|TransitiveTarget\w*\|TargetLoading\w*\|LoadedPackage\w*\|SkyframePackage\w*\|BzlCompile\w*\|StarlarkFile\w*\|CompiledBuildFile\w*\|DirectoryListing\w*\|File\w*\|DirectoryTree\w*\|RepositoryMapping\w*\|ProjectFiles\w*\|WorkspaceName\w*\|ToplevelStarlarkAspect\w*)`<br>`lib/cmdline/` |
| execution-and-action-graph | `lib/actions/`<br>`lib/skyframe/`<br>`lib/buildtool/`<br>`com/google/devtools/build/skyframe/` |
| runtime-and-ui | `lib/runtime/`<br>`lib/bazel/`<br>`lib/events/`<br>`lib/outputfilter/`<br>`lib/util/io/`<br>`lib/buildeventstream/` |
| base-libraries | `lib/util/`<br>`lib/concurrent/`<br>`lib/vfs/`<br>`lib/collect/`<br>`lib/io/`<br>`lib/clock/`<br>`lib/graph/`<br>`lib/supplier/`<br>`lib/versioning/`<br>`lib/bugreport/`<br>`lib/network/`<br>`lib/causes/`<br>`com/google/devtools/common/options/` |
| other | `.` |
<!-- END GENERATED RULES -->
