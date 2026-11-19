# workerd (stress project)

cloudflare/workerd @6ee5f171, target `build //src/workerd/server:workerd` (corpus/projects.json).
Tried on 2026-11-18 at the user's request. workerd pins Bazel 9.3.1 (`.bazelversion`); the oracle
binary is called directly, so the pin is not consulted and no fix-up was needed.

Tools:

- `scripts/workerd/run <oracle|blaze> [--sync] <command> ...`: runs either binary on a copy of the
  checkout (oracle: `~/.cache/blaze/work/workerd`, output user root `oracle-out/ur-workerd`;
  blaze: `work/workerd-blaze`, `oracle-out/ur-workerd-blaze`), clean env, 20G scope.
- `scripts/workerd/compare-repos [--ascii-readme]`: `scripts/w5r/compare-repos` against the oracle
  build's output base, without adding workerd to that tool's `--project all`.
- `scripts/w3/oracle-repos --project workerd --repo-output-base <base>` now works for workerd
  (tolerates the oracle's `mod graph` exit 2, per-repo fallback when a failing extension breaks
  `dump_repo_mapping`, root-module extension repos `+rule+name`). Output:
  `~/.cache/blaze/w3/workerd.repos.json`.
- `scripts/w3/query-compare --project workerd`. Note: `--project all` of the w3/w5r/oracle tools
  now includes workerd (it is in corpus/projects.json); brotli/re2 regression checks run per
  project.

## Oracle baseline (Bazel 9.3.2)

| command | result |
| --- | --- |
| `mod graph --output=json` | exit 2, graph printed (83 KB). `ERROR: error evaluating module extension @@rules_rust+//crate_universe:extension.bzl%crate`: protobuf's MODULE.bazel uses crate_universe `crates` without a lockfile, which rules_rust forbids for non-root modules ("Results may be incomplete as 1 extension failed."). Same failure in `oracle-repos`' `mod` commands. |
| `query //src/workerd/server:workerd` | exit 0, 1 target (`rust_binary`) |
| `query deps(...)` (`--keep_going`) | exit 3, 46,370 targets: `@@+http_archive+v8//:icu/run_metagen` references the nonexistent package `@@+http_archive+v8//external` (follows every `select()` branch, like re2's libpfm) |
| `cquery deps(...) --output=label_kind` | exit 0, 38,891 configured targets |
| `aquery deps(...) --output=jsonproto ...` | **exit 8 ("build interrupted") on a fresh output base**, reproducibly, after analysis (44,464 targets configured). On the output base of a finished build: exit 0, 478 MB, 13,416 actions. |
| `build //src/workerd/server:workerd --jobs=10` | **success**, 1,090 s, 9,713 actions (6,379 linux-sandbox, 3,333 internal, 1 local), no network or host-toolchain problems (toolchains_llvm, rust, node, python all downloaded hermetically). |

Recorded as `oracle-9.3.2/workerd` (`~/.cache/blaze/oracle-records/oracle-9.3.2/workerd`,
summary `corpus/oracle/workerd.json`) with the cheap surfaces: query_label_kind and
query_jsonproto 46,371 items, cquery_label_kind 38,891, cquery_jsonproto 2, aquery_actions 13,416.
How: recorded under label `oracle-9.3.2-workerd` (its own output user root; recording under
`oracle-9.3.2` would wipe `rec-oracle-9.3.2`, which re2's repos.json points into), in a 20G scope,
then copied. The aquery output is the one from the built output base (manifest notes say so).
mod_graph/lockfile surfaces are FAILED in the record because the oracle exits 2; the raw outputs
are there. Build surfaces were skipped (the build above was run separately).

## Blaze, phase by phase

| phase | how | result |
| --- | --- | --- |
| mod graph | `blaze-fetch-dev ... mod graph --output=json` (new `mod` verb: the fetching evaluator, so non-registry overrides are really fetched) | **byte-identical JSON, same exit code 2, identical MODULE.bazel.lock** — with the README fix-up below. Without it: exit 37, no graph. |
| fetch | `scripts/workerd/compare-repos --ascii-readme` (1,017 repos of the oracle build) | exit 0 in 121 s; **1,013/1,017 byte-identical**; the 4 others: telemetry report (byte strings, below), `local_config_cc_toolchains` (the dev tool does not read `.bazelrc`; identical when given its `--repo_env` flags), `com_cloudflare_lol_html` and `python_3_13` (symlink target / `hash(workspace_root)` of a different workspace path; artifacts of the comparison setup). Hidden lockfile 44/44; workspace lockfile differs only in telemetry `last_notice` (compare-repos deletes the committed lockfile, so no facts). |
| query | `scripts/w3/query-compare --project workerd` (OracleRepos) | **46,371/46,371 MATCH** on query_label_kind and query_jsonproto (pattern and `deps()`), exit codes 0/3 equal. The main `blaze query` prints the same pattern result. |
| cquery | main binary (real repos now wired by the integration lead) | blocked: panics `SkyframeExecutor.toCanonicalLabelString` (fixed in blaze_build_config, needs a one-line switch in blaze_sky_top); with that switch in a private probe build, `deps(//src/workerd/util:entropy)` gives the oracle's 1,074 labels and kinds, but the target configuration differs (Starlark flags from `.bazelrc` are dropped); `deps(//src/workerd/server:workerd)` panics on `HeaderInfo` hashing, then on the missing genrule factory. |
| aquery | — | not reached (needs cquery's analysis). |

### The README emoji: byte-string semantics (blocks true parity)

`aspect_tools_telemetry` (pulled in by aspect_rules_js/ts/esbuild) hashes the first 4 lines of the
workspace README with a pure-Starlark SHA-1: `for char in input.elems(): ord(char)` where `ord` is
a dict lookup over `"\000".."\377"` (bazel_lib `strings.bzl`). workerd's README starts with an
emoji. Bazel's strings are Latin-1 internal strings holding UTF-8 bytes, so `elems()` yields 4
byte-chars that are all in the table; starlark-rust yields one code point that is not, `ord`
returns None, and `None << 8` fails. The telemetry repo then fails to fetch, and every extension
that loads `@aspect_tools_telemetry_report` fails with it (mod graph exit 37). This is the
code-point ruling of decisions.md (2026-11-17); root has started a design study for Bazel's byte
strings. Until then every Blaze probe runs on a copy whose README first line has the emoji
removed (`scripts/workerd/compare-repos --ascii-readme`, `work/workerd-blaze`); never in
`~/.cache/blaze/corpus`. The telemetry repo's `report.json` therefore differs from the oracle's by
design.

### Scope of the byte-string dependence (survey of 2026-11-18)

Question: is the telemetry hash the only place in workerd's closure whose result depends on
Bazel's byte semantics? Read-only survey of the checkout, the 1,017 external repositories of
the oracle's output base (symlinks followed), the embedded `@bazel_tools` and `@_builtins`, the
396 registry files of the lockfile and the files recorded as repository inputs (1,016
`.marker` / `.recorded_inputs` files). **Answer: yes, it is the only one.**

| site | operation | non-ASCII input | output | reached by |
| --- | --- | --- | --- | --- |
| `aspect_tools_telemetry+` 0.5.2 `collectors/dedup.bzl:88` (`repository_ctx.read(readme_file).split("\n")[:4]`), `sha1.bzl:37-38` (`for char in input.elems(): message_bytes_list.append(ord(char))`, bazel_lib's `ord` is a dict over `"\0".."\377"`), `sha1.bzl:79` (`block[t*4] << 24`) | `elems` + per-unit table lookup | `//:README.md` line 1: the emoji F0 9F 91 B7 (the file's only 4 non-ASCII bytes) | `id_day` in the telemetry `report.json` | fetching alone (`@aspect_tools_telemetry_report` is loaded by `aspect_rules_js+/npm/defs.bzl:4` and `aspect_rules_esbuild+/esbuild/extensions.bzl:3`), so every build that loads rules_js or rules_esbuild |

Touching non-ASCII text but with the same result under either semantics, provided I/O
converts bytes correctly: comments (94 files), docstrings (21), `doc = "..."` attribute strings
(6 files), two metadata strings (package_metadata SPDX `full_name`, rules_android
`copyright_notice`), one `rctx.template` without substitutions (rules_kotlin
`capabilities.bzl`), one native `ctx.patch` (V8 patch 0013), two non-ASCII file names in a V8
`glob(["test/**"])` (not in workerd's build graph), `json.decode` of `@types/node`'s
`package.json` (only the ASCII `bin` is used), an ASCII-needle `in` test on
`markupsafe/__init__.py`, and workerd's own `build/python_metadata.bzl:11-12`
(`s += chr(int(chunk, 16))` then `base64.encode(s)`: chars `\200`..`\377` built from escapes;
correct as long as `"\NNN"` stays one unit).

Counts: `.bzl` 3,128 scanned / 52 with non-ASCII bytes; `BUILD*` 7,971 / 67; `MODULE` 270 / 2;
`REPO.bazel` 920 / 0; `WORKSPACE*` 386 / 0; bazelrc 210 / 37 (comments). Only 12 Starlark files
have non-ASCII inside a non-docstring string literal, none of them measured or indexed.
Builtins zip 28 files / 0; registry files 396 / 0; recorded file inputs 165 / 3 (README, the V8
patch, kotlin capabilities).

Latent (byte-sensitive code whose input is ASCII in this workspace): aspect_rules_js
`utils.hash` (`str(hash(s))`, Java `String.hashCode`) over pnpm-lock.yaml / package.json /
.npmrc; workerd `sysroot.bzl:27` md5sums and `.lower()`; toolchains_llvm `_strip_jsonc`;
rules_python `parse_requirements_txt`; `hash(path)` in rules_cc / rules_rust / protobuf /
skylib; package_metadata's `purl` string tests (they assert byte semantics, test-only).

Not checked: `execute().stdout` values other than through the generated files (all ASCII),
`oci.pull` manifests and repositories the oracle did not fetch, and module-extension reads that
are not recorded (covered by grep for read sites, not by a trace).

So for workerd the migration is needed for exactly one hash; the referee's protocol numbers for
cquery and aquery are blocked by it only because the recorder uses the pristine README.

## Triage

| # | phase | crate | Java | gap | first error | status |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | mod | blaze_bzlmod | `ModuleFileFunction.maybePatchModuleFile` / `PatchUtil.applyToSingleFile` | single_version_override patches applied to MODULE.bazel (rules_rust, toolchains_llvm) | `not ported yet: PatchUtil.applyToSingleFile` | fixed |
| 2 | mod | blaze_repo_rules (`setup.rs`) | `ModuleFileFunction.getModuleFile` (REPOSITORY_DIRECTORY for non-registry overrides) | bzlmod's builtin lookup only knew local_path_override; 6 archive/git overrides (boringssl, fast_float, fp16, highway, perfetto, zlib) | `not ported yet: RepositoryFetchFunction.compute: repository @@boringssl+ defined by http_archive` | fixed (`RepositoryDirectoryFunctionLookup`; kept by the lead's `prepare_repository_functions_in`, so the runtime has it too) |
| 3 | mod | starlark-rust (patch 0005, part) | `StringModule.elems` | `elems()` returned an iterator, not a list | rules_python `parse_requirements_txt.bzl:59 content.elems()[:]`: ``Operation `[::]` not supported on type `iterator` `` | fixed |
| 4 | mod | blaze_repo_rules | `ModuleExtensionContext.isIsolated` (`enableOnlyWithFlag`) | field must not exist without the flag | gazelle `getattr(module_ctx, "is_isolated", False)`: `'module_ctx' value has no field or method 'is_isolated'` | fixed |
| 5 | mod/fetch | starlark-rust (decisions.md string ruling) | Latin-1 internal strings | byte-string semantics | telemetry `sha1.bzl:81`: ``Operation `<<` not supported for types `NoneType` and `int` `` | open, root's design study (workaround: ASCII README) |
| 6 | fetch | blaze_repo_fetch | Commons Compress `TarUtils.parseOctal` | blank tar mtime field is 0 | 2 crates.io archives (`precomputed-hash-0.1.1`, `pathdiff-0.1.0`): `numeric field was not a number:  when getting mtime` | fixed |
| 7 | fetch | starlark-rust (patch 0019) | `Dict.get` / `Dict.setdefault` (`default` is `named = true`) | named `default` | toolchains_llvm `llvm_distributions.bzl:236 rctx.attr.sha256.get(key, default = "")` | fixed |
| 8 | fetch | blaze_packages (`builtins/stubs.rs`) | `PyBuiltins.regexMatch` | loading-phase `py_builtins` stub had no `regex_match` (rules_python `py_internal` in repo rules) | 2 pip whl repos: `not ported yet: py_builtins.regex_match` | fixed (`PyBuiltinsLoading`; the analysis env keeps blaze_rules' full object) |
| 9 | query | blaze_packages | `AspectPropagationEdgesSupplier.ALL_TOOLCHAINS` | `toolchains_aspects = ["*"]` | rules_cc `cc_shared_library.bzl`: `not ported yet: AspectPropagationEdgesSupplier.ALL_TOOLCHAINS` | fixed |
| 10 | query | blaze_packages (`types/type_.rs`) | `Type.ListType.convert` (a `Dict` is an `Iterable` of its keys) | dict given to a list attribute | ncrypto `local_defines = {"NAME": select(...)}`: `expected value of type 'list(string)' ... (dict)` | fixed |
| 11 | query | blaze_query | `AspectDefinition.satisfies` | required providers of Starlark aspects; also restored Java's order (satisfies before collecting aspect deps) | `not ported yet: AspectDefinition.satisfies: required providers of aspect ...upb_minitable_proto_library_aspect` | fixed (native aspects still `unported!`) |
| 12 | cquery | blaze_build_config + blaze_sky_top | `SkyframeExecutor.getFlagAliases` / `toCanonicalLabelString` | flag_alias() labels resolved with the defining module's full repo mapping | `not ported yet: SkyframeExecutor.toCanonicalLabelString: flag_alias() in module rules_python` | ported in blaze_build_config (`get_flag_aliases_from_dep_graph`); **blaze_sky_top must call it** (integration lead) |
| 13 | cquery | blaze_runtime (`dispatcher.rs`) | `BlazeOptionHandler.parseStarlarkOptions` | Starlark flags from the command line and `.bazelrc` (`--//build/config:...`, `--@v8//:...`, `--@rules_rust//...`) are silently dropped; 21 user-defined options missing from the target configuration (checksum 6d8e5e8 vs oracle 13e714b) | (no error: wrong configuration) | open, integration lead; should fail loudly until ported |
| 14 | cquery | blaze_build_config (`skyframe.rs`) | `BuildConfigurationKeyProducer.findBuildOptionsScopes`, `BuildOptionsScopeFunction`, `BuildOptionsScopeValue` (~370 Java lines) | flag scoping for configurations with Starlark options | `unported!` as soon as #13 lands or a transition sets a Starlark flag | open (mine; not ported because it cannot be validated before #13) |
| 15 | cquery | blaze_rules (`cpp/header_info.rs`) | Java `Object.hashCode`/`equals` (identity) | `HeaderInfo` has no `write_hash`, so structs holding it cannot go into a depset | rules_rust `rustc.bzl:1802 depset(deps)`: ``Value of type `HeaderInfo` is not hashable`` | open (blaze_rules owner); identity hash verified in the probe build |
| 16 | cquery | blaze_rules / blaze_analysis | `BazelGenRule` / `GenRuleBase` configured target factory | genrule analysis not ported (40 genrules in workerd's deps, V8/ICU) | `RuleConfiguredTargetFactory.create: no native factory registered for BazelGenRule` | open (blaze_rules owner) — the furthest blocker |
| 17 | cquery | blaze_sky_top | `SkyframeErrorProcessor.processAnalysisErrors` | analysis errors end in a panic | `not ported yet: SkyframeErrorProcessor.processAnalysisErrors` | open (integration lead) |
| 18 | oracle | — | — | aquery exits 8 on a fresh output base | `ERROR: build interrupted` | oracle behaviour; recorded aquery comes from the built base |

Counts at the end of the aspects wave: 29 items (the table continues under "Triage,
continued"); #12 to #17 were fixed between the two waves; open: #5 (root, byte strings), #18
(oracle behaviour) and the build loop (#29 onwards, see "Build loop: how to continue"); #24 to
#28 were fixed during the wave.

## Aspects wave (2026-11-18, aspects lead)

State of the tree before the wave (HEAD 12ec0c9; #12 to #17 of the table above are fixed): the
full target stops at `AspectResolutionHelpers.computePropagatingAspects` (aspects on the `deps`
of protobuf's `upb_c_proto_library` targets, and on `launcher` of the esbuild toolchain).

New tools:

- `scripts/workerd/triage <labels> <outdir>`: `blaze cquery 'deps(<label>)'` per label, with the
  first failure of each; `triage kinds` lists one label per rule kind of the oracle's cquery
  recording (135 kinds). `triage --aquery` runs the comparison below per label.
- `scripts/workerd/aquery-compare '<expr>' [--full]`: Blaze's aquery of any part of the closure
  against the oracle's recorded actions with the same keys (the referee's own extractor); a
  development aid, not a score.

Before any aspect work, with the binary of HEAD 12ec0c9 (`triage kinds`, 135 labels):

- cquery: 124 of 135 labels analyse to the end (81,779 result lines); 10 stop at the aspect
  `unported!` above and one label was quoted wrongly by the first version of the tool. No other
  unported native is reached outside aspect-carrying subtrees.
- aquery on ten of the largest subtrees that pass (`triage --aquery`; 8 ran, 2 exit 8, next
  point): 7,247 actions printed, 7,142 with a key in the oracle record, 6,890 identical, 252
  different. Every difference is the `actionKey` of a `SourceSymlinkManifest` (126) or
  `SymlinkTree` (126) action: the referee's allow-listed `runfiles-manifest-action-key` (the key
  fingerprints absolute execroot paths). 105 actions (two subtrees) are not in the oracle record
  because the label was queried in the target configuration, which the real build reaches in
  another one.
- aquery exits 8 ("build interrupted") for `deps(@v8//:code_generator)` and the pyodide subtree,
  **as the oracle does on a fresh output base** (#18): `--include_file_write_contents` makes
  `SourceManifestAction` read the target of an unresolved symlink artifact
  (`UnresolvedSymlink` actions of rules_python's venv) from disk; before a build it is not
  there. `mnemonic("^SourceSymlinkManifest$", deps(@v8//:code_generator))` with that flag is the
  smallest reproducer. So the protocol result for workerd's aquery surface is exit 8 on both
  sides; the action list can only be compared on a built output base.

### After the aspect port (working tree on HEAD cf0819b, 2026-11-18)

All numbers below are from `target/debug/blaze` built from the working tree, on the
ASCII-README copy, with the tools above (the referee's extractors; development aids, not scores).

| surface | result |
| --- | --- |
| analysis | `Analyzed target //src/workerd/server:workerd (736 packages loaded, 44464 targets configured, 9 aspect applications)`; the oracle on a fresh server prints 737 / 44464 / 9. The 737 packages are the same on both sides (oracle: `dump --skyframe=deps --skykey_filter='^PACKAGE:'`; Blaze: a throwaway binary printing each `doneReadPackage`); the count differs because of #26 |
| cquery_label_kind (`cquery-compare`) | 38,892 of 38,892 items, the `__order__` item included |
| cquery_jsonproto | 4 of 4 items, both order items included |
| aquery_actions (`aquery-compare --full --order --make-unresolved-symlinks`) | 13,416 actions on both sides, none missing, none extra; 13,160 identical in every field; 256 differ only in `actionKey`: 128 `SourceSymlinkManifest` and 128 `SymlinkTree` runfiles actions (allow-listed kind `runfiles-manifest-action-key`). All nine `__order__` items equal: actions 13,416, action ids 13,416, artifacts 49,857, depSetOfFiles 22,250, pathFragments 55,226, targets 2,242, configuration 4, ruleClasses 48, aspectDescriptors 2 |
| build of sub-targets | `build //src/workerd/server:malloc --jobs=8`: 314 actions (169 linux-sandbox); its 485 objects, archives, shared objects, module maps, `.d` and `.params` files are byte-identical to the oracle's files at the same paths. Also complete (exit 0): `//src/rust/cxx:cxxbridge-macro` (378 actions), `//src/workerd/server:workerd_capnp_rust_gen` (205), `@capnp-cpp//src/capnp:capnpc-capnp` (156), the cxx bridge genrule (118), `@v8//:icu/generated_torque_files` (794), the tokio / rustls / hyper / regex / serde_json crates (173) and eight first-party Rust crates (203). Stopped by #27: `@v8//:code_generator`, `//src/pyodide:pyodide_static@tsproject`; by #28: `//src/workerd/server:workerd_capnp_rust` and three more |
| full build | `build //src/workerd/server:workerd --jobs=10` (binary sha256 `45247cb92712`, HEAD d294400 plus the working tree): analysis completes with the oracle's line (737 packages loaded, 44464 targets configured, 9 aspect applications), then the process stops after 52 s at #29 |

Referee protocol: `scripts/oracle/record --project workerd` runs on a pristine copy (README with
the emoji), so Blaze's cquery and aquery commands fail there at `aspect_tools_telemetry` until
byte strings are ported (#5); the label recorded for this state is `blaze-aspects-2`.

### Triage, continued (aspects wave)

| # | phase | crate | Java | gap | first error | status |
| --- | --- | --- | --- | --- | --- | --- |
| 19 | cquery | blaze_packages, blaze_analysis, blaze_sky_analysis, blaze_sky_top, blaze_cquery | `AspectFunction`, `AspectKeyCreator`, `AspectCollection`, `AspectResolutionHelpers`, `ConfiguredAspect`, `AspectContext`, `MergedConfiguredTarget`, `StarlarkAspectFactory`, producers (see crates/blaze_analysis/DESIGN.md, "Aspects wave") | aspects | `not ported yet: AspectResolutionHelpers.computePropagatingAspects` (10 of 135 rule kinds) | fixed |
| 20 | cquery | blaze_starlark (`label.rs`) | `Label.getRelative` (`BazelModuleContext.ofInnermostBzlOrThrow(thread).repoMapping()`) | `Label.relative()` only worked while a .bzl file loads | protobuf `upb_proto_library_internal/aspect.bzl:197`: `Label.relative() requires a repository mapping` | fixed |
| 21 | cquery (order) | blaze_sky_analysis (`dependency_map_producer.rs`) | `ConfiguredTargetAndDataProducer.step` with `BaseTargetPrerequisitesSupplier` | the batch that requests one round's configured targets also requested the base target's dependencies of an aspect, adding edges Java does not have | 26 of 38,891 cquery results out of order (`deps(esbuild_toolchain, 1)`: 27 results, oracle 6) | fixed |
| 22 | (order, latent) | blaze_packages (`aspect/aspect_definition.rs`) | `AspectDefinition.Builder.toolchainTypes` (`HashSet`) | toolchain types of an aspect in insertion order, Java: `HashSet` order | none on workerd | fixed (Java order from `ToolchainTypeRequirement`'s record hash) |
| 23 | build | blaze_sky_top (`build_view.rs`) | `BuildView.setArtifactRoots` | `ArtifactFactory.packageRoots` never set | `NullPointerException: packageRoots` in `CppCompileAction` `.d` input discovery | fixed |
| 24 | build | blaze_sky_exec (execution owner) | `TrackSourceDirectoriesFlag` | default assumed false; Bazel's is true | 676 `input 'sysroot' ... is a directory; dependency checking of directories is unsound` warnings for one cc sub-target (oracle: none in the whole build) | fixed by the execution owner (d294400): 0 warnings |
| 26 | UI | blaze_runtime (dispatcher / build wiring) + blaze_sky_top (`build_tool.rs`) | `SkyframeExecutor.preparePackageLoading` (`packageProgress.reset()`, SkyframeExecutor.java:1742) before `BlazeOptionHandler.parseStarlarkOptions` | Blaze loads the packages of the Starlark flags of the command line and `.bazelrc` (10 on workerd) in an evaluator of their own before the build, resets the counter when the build starts (`build_tool.rs`), then loads 9 of them again; `@@rules_rust+//` (only needed for `--@rules_rust//:extra_rustc_flag` etc.) is never loaded by the build itself. Java loads each once, in the one graph, after the reset | `736 packages loaded` vs the oracle's `737` | fixed on this side (the receiver counts a package once per command, the reset in `build_tool.rs` is gone): Blaze prints 737 / 44464 / 9 like the oracle; with a persistent server the dispatcher needs the reset at its `syncPackageLoading` port |
| 27 | build | blaze_sky_exec (execution owner) | `FileArtifactValue.createForUnresolvedSymlink` | output metadata of `declare_symlink` outputs | `not ported yet: FileArtifactValue.createForUnresolvedSymlink` (`build @v8//:code_generator`, `//src/pyodide:pyodide_static@tsproject`; 14 `UnresolvedSymlink` actions in the oracle's build) | fixed by the execution owner (d294400) |
| 28 | build | blaze_buildtool / blaze_sky_exec (execution owner) | `ActionExecutionFunction.PackageRootResolverWithEnvironment.findPackageRootsForFiles` | the `ContainingPackageRootLookup` seam is never supplied (`execution_tool.rs`: `None`) | `not ported yet: ... (no ContainingPackageRootLookup given to the executor)` (`build //src/workerd/server:workerd_capnp_rust ...`) | fixed by the execution owner (working tree after d294400: the sub-target builds) |
| 29 | build | blaze_exec (execution owner) | `RunfilesTreeUpdater.updateRunfiles` | the local spawn runner cannot create a tool's runfiles tree | `not ported yet: RunfilesTreeUpdater.updateRunfiles` at `local_spawn_runner.rs:87`: the one `local`-tagged action of the build, `Genrule @@+http_archive+v8//:generated_inspector_files` (its tool is the `py_binary` `@v8//:code_generator`) | open, reported to root: **stops the full build after 52 s** |
| 25 | build (`--aspects`) | blaze_sky_exec, blaze_runtime (`commands/build.rs`), blaze_sky_top | `BuildDriverFunction` for `TopLevelAspectsKey`, `AspectCompletor`, `SkyframeBuildView.analyzeAndExecuteTargets` (`BuildDriverKey.ofTopLevelAspect`), `ExecutionTool.determineSuccessfulAspects` | top-level aspects on the build path | `unported!` (not reached by workerd's default configuration) | fixed: scenario `aspects` 21 of 22 steps in the masked view (the last one prints `3 action cache hit` because Blaze starts a cold server per step; the oracle prints the same line when its server is restarted) |

## Build loop: how to continue (handover, 2026-11-18)

State: analysis, cquery and aquery of `//src/workerd/server:workerd` match the oracle; the
build loop is the remaining work. Everything below works from a cold start.

Run:

```
scripts/cargo-capped build -p blaze
cp target/debug/blaze <somewhere outside target/>/blaze     # other agents rebuild target/debug/blaze
BLAZE=<copy> RUST_BACKTRACE=1 scripts/workerd/run blaze build //src/workerd/server:workerd --jobs=10 \
    > ~/.cache/blaze/work/workerd-logs/blaze.build.stdout 2> ~/.cache/blaze/work/workerd-logs/blaze.build.log
```

- `scripts/workerd/run` (see its header) uses the work copy `~/.cache/blaze/work/workerd-blaze`
  (README first line without the emoji: byte strings, #5) and the output user root
  `~/.cache/blaze/oracle-out/ur-workerd-blaze`, in a 20G systemd scope. Outputs and the action
  cache persist, so a rerun continues after the last stop. Tell root before a full build.
- A stop is a panic `not ported yet: <Class.method>` (every gap is loud; `--keep_going` cannot
  step over a panic), an `ERROR:` the oracle's log does not have, or a wrong output. One stop at
  a time: find the Java at the pin, port it literally in the crate of the class, rebuild, rerun.
  A sub-target reproduces a stop quickly (`... build <label>`); `scripts/workerd/triage` and
  `scripts/workerd/aquery-compare '<expr>'` give per-kind analysis and action checks.
- Name the binary in every number: `sha256sum <copy> | cut -c1-12`, `git rev-parse --short HEAD`
  and whether the tree was dirty.

Reference (read-only): the oracle's log `~/.cache/blaze/work/workerd-logs/oracle.build.log`
(1,090 s, `9713 processes: 3333 internal, 6379 linux-sandbox, 1 local`, exactly one WARNING,
about sqlite3.h `manifest.tags`) and its built tree
`~/.cache/blaze/oracle-out/ur-workerd/691d652ad055a3e15332bab73c786266/execroot/_main/bazel-out`.
Blaze's tree: `~/.cache/blaze/oracle-out/ur-workerd-blaze/8701369257f4bea6fc3e248fd7d7bf44/execroot/_main/bazel-out`.

Comparing when the build completes:

1. Exit code and the final `INFO: N processes: ...` line against the oracle's.
2. `sha256sum` of `bazel-out/k8-fastbuild/bin/src/workerd/server/workerd` on both sides.
3. Every regular file under Blaze's `bazel-out` against the same path in the oracle's (same
   content or not, by extension; files only one side has). Known, explained differences so far
   (4,155 of 4,253 files built by sub-target builds were identical): `*.runfiles_manifest` and
   `*.runfiles/MANIFEST` (absolute paths), `stable-status.txt` / `volatile-status.txt`, and the
   objects and archives that ring's cargo build script compiles in its `_bs.out_dir`.
4. WARNING / ERROR lines of the two logs.
5. Then the referee's build surfaces can be recorded for workerd on both sides (`build_stderr`,
   `execlog`, `bep`, `outputs`); the oracle's record has none yet.

Guard rails for every change in shared code: `scripts/oracle/record --binary <copy> --label
<new label> --project brotli` and `--project re2`, `scripts/oracle/scoreboard <label> --json`,
`scripts/gate/compare-scores corpus/scores/blaze-ef66f55e904e.referee2.json <file>` must print
`regressed 0`; `scripts/gate/run-tests` + `compare-tests` must lose no name;
`scripts/w4/probe compare-all` totals (39 scenarios, 720 steps: full 102, masked 570, messages
239 before the `aspects` scenario was added) must not drop; `scripts/workerd/cquery-compare` and
`aquery-compare 'deps(//src/workerd/server:workerd)' --full --order --make-unresolved-symlinks`
(memory-capped scope) must stay at 38,892 / 4 and 13,160 + 256.

## Regression checks (brotli, re2)

After all fixes, on the working tree of 2026-11-18 (HEAD bd9694d plus uncommitted changes):

- `scripts/w3/query-compare --project brotli`: 162/162 label_kind, 162/162 jsonproto, exit codes
  equal; `--project re2`: 1091/1091, 1091/1091.
- `scripts/w5r/compare-repos --project brotli`: 67/67 repos, lockfile MATCH, hidden 11/11;
  `--project re2`: 81/81, lockfile MATCH, hidden 20/21 (`provisioning_profile_repository_extension`:
  `apple_common.dotted_version` is a loading stub; untouched by these changes).

Aspects wave (2026-11-18), binary sha256 `7546139d63d8` built from HEAD 4e9d079 plus the
working tree (committed by root as d294400):

- Referee, label `blaze-aspects-4` (dirty tree, binary copied out of `target/`, so the label is
  evidence for the wave and not an accepted score): `scripts/gate/compare-scores
  corpus/scores/blaze-ef66f55e904e.referee2.json`: rows 39, gained 3, regressed 0, PASS; brotli
  and re2 are unchanged at 13,850 matching items (23 of 26 surfaces MATCH). Earlier points of
  the wave: `blaze-aspects-1`, `-2`, `-3`, each PASS with 0 regressed.
- Test gate on the working tree (`scripts/gate/run-tests`, `compare-tests` against
  `docs/state/tests-baseline.json.gz`): pass 6,345 (baseline 5,487), fail 0, ignored 2,029;
  retained 5,487, lost 0, absent 0, new-pass 851, recovered 7.
- `scripts/w4/probe compare-all`: 39 scenarios, 720 steps; full 102, masked 570, messages 239,
  equal to the saved transcripts of the previous binary. New scenario `aspects` (recorded on the
  oracle): 19 of 22 steps matched in the masked view at that binary; 21 of 22 after #25 was
  wired (label `blaze-aspects-5` for brotli and re2 at that point: 0 regressed, PASS).

## Next for workerd

Items 1 to 3 of the earlier list (#12 to #16) are done; the whole target analyses and cquery and
aquery match the oracle on the ASCII-README copy. In order:

1. Execution owner: `RunfilesTreeUpdater.updateRunfiles` for the local spawn runner (#29), the
   first stop of the full build (#24, #27 and #28 are fixed). The oracle needs about 18 minutes
   and 9,713 actions for the build; more execution gaps may follow, each one a loud stop.
2. Byte strings (#5): one site in the whole closure (the survey above). It gates every referee
   number of workerd's cquery and aquery surfaces, because `scripts/oracle/record` uses the
   pristine README. Under the protocol today (label `blaze-aspects-4`): query surfaces partly
   match (the telemetry repository fails under `--keep_going`), cquery and aquery FAILED.
3. Top-level aspects on the build path (#25) are wired; what is left there is the result
   display after an analysis failure (`build.rs`, the `build_result_listener` path passes no
   aspects) and the native aspect class map for `--aspects=<NativeName>`.
4. After a full build exists: record the build surfaces (`build_stderr`, `execlog`, `bep`,
   `outputs`) for workerd on both sides; none is recorded for the oracle yet either.
