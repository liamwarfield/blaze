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

Counts: 18 items; 11 fixed (1–4, 6–12 with #12's sky_top call pending), 7 open (#5 root,
#13/#17 integration lead, #15/#16 blaze_rules, #14 blaze_build_config, #18 oracle-only).

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
| build of a sub-target | `build //src/workerd/server:malloc --jobs=8`: 314 actions (169 linux-sandbox); its 485 objects, archives, shared objects, module maps, `.d` and `.params` files are byte-identical to the oracle's files at the same paths |

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
| 24 | build | blaze_sky_exec (execution owner) | `TrackSourceDirectoriesFlag` | default assumed false; Bazel's is true | 676 `input 'sysroot' ... is a directory; dependency checking of directories is unsound` warnings for one cc sub-target (oracle: none in the whole build) | open, reported to root |
| 26 | UI | blaze_runtime (dispatcher / build wiring) + blaze_sky_top (`build_tool.rs`) | `SkyframeExecutor.preparePackageLoading` (`packageProgress.reset()`, SkyframeExecutor.java:1742) before `BlazeOptionHandler.parseStarlarkOptions` | Blaze loads the packages of the Starlark flags of the command line and `.bazelrc` (10 on workerd) in an evaluator of their own before the build, resets the counter when the build starts (`build_tool.rs`), then loads 9 of them again; `@@rules_rust+//` (only needed for `--@rules_rust//:extra_rustc_flag` etc.) is never loaded by the build itself. Java loads each once, in the one graph, after the reset | `736 packages loaded` vs the oracle's `737` | open, reported to root (fix: one evaluator, or reset before the flags are parsed and count a package once per command) |
| 25 | build (`--aspects`) | blaze_sky_exec (execution owner) | `BuildDriverFunction` for `TopLevelAspectsKey`, `AspectCompletor` | top-level aspects on the build path | `unported!` (not reached by workerd's default configuration) | open, reported to root |

## Regression checks (brotli, re2)

After all fixes, on the working tree of 2026-11-18 (HEAD bd9694d plus uncommitted changes):

- `scripts/w3/query-compare --project brotli`: 162/162 label_kind, 162/162 jsonproto, exit codes
  equal; `--project re2`: 1091/1091, 1091/1091.
- `scripts/w5r/compare-repos --project brotli`: 67/67 repos, lockfile MATCH, hidden 11/11;
  `--project re2`: 81/81, lockfile MATCH, hidden 20/21 (`provisioning_profile_repository_extension`:
  `apple_common.dotted_version` is a loading stub; untouched by these changes).

## Next for workerd

1. blaze_sky_top: call `get_flag_aliases_from_dep_graph` (#12).
2. blaze_runtime: port `parseStarlarkOptions` (#13); then blaze_build_config flag scoping (#14).
3. blaze_rules: identity `write_hash` for `HeaderInfo` and other native infos (#15); genrule (#16).
4. Byte-string semantics (#5) for parity of the telemetry repo and anything else that walks
   non-ASCII text byte by byte.
