# Review: small ports of checkpoints 43 to 46 (first review)

Date 2026-11-27. Commit under review: `8ecc520`. Binary used for probes:
`~/.cache/blaze/work/root-runs/blaze-8ecc520`; oracle `bazel-9.3.2-linux-x86_64`.
Probe workspaces, scripts and outputs: `~/.cache/blaze/work/review-small-ports/`
(`probe.sh`, `probew.sh`, `probed.sh`, `probef.sh`, results under `res/`). All servers were
shut down after each probe. No Cargo was run.

| Item | Verdict at `8ecc520` | Blocking findings |
|---|---|---|
| 1. Hash functions (SHA-1, Git SHA-1, BLAKE3) | PASS | none |
| 2. WebAssembly in repository rules | STOP | W-1, W-2 |
| 3. `starlark_doc_extract` | PASS | none |
| 4. `--proto:rule_classes`, Android and J2ObjC fragments, feature flags | PASS | none |
| 5. Auto output filter, glob interrupt (skim) | PASS | none |

## 1. Hash functions: PASS

Read: `blaze_vfs/src/digest_hash_function.rs` (whole), `blaze_optdefs/.../digest_hash_function.rs`,
`blaze_remote/src/util/digest_util.rs` (whole), `blaze_exec/.../cache/digest_utils.rs` (digest
paths), `UnixFileSystem::get_fast_digest/get_digest`, the file system choice in
`blaze_runtime/src/runtime.rs:1036-1049`, `DownloadCache.getChecksum`, and the Rust counterpart of
every Java caller of `getDigestFunction()` outside `remote/` (bundled file system, `BinTools`,
`VirtualActionInput.writeTo`, bzl compile and load, query and genquery hash function). Against
`vfs/DigestHashFunction.java`, `GitSha1HashFunction.java`, `vfs/bazel/*.java`,
`remote/util/DigestUtil.java`, `vfs/DigestUtils.java`, `bazel/BazelFileSystemModule.java`.

Probes (same two-genrule build on both sides, clean output base, `--disk_cache`,
`--execution_log_json_file`, then `aquery --output=jsonproto`):

- `--digest_function=sha1` and `blake3`: the execution logs are equal except times and memory
  peaks; the disk cache holds the same eight files (`ac/..`, `cas/..` for SHA-1;
  `blake3/ac/..`, `blake3/cas/..` for BLAKE3); the `aquery` output is equal.
- `--digest_function=gitsha1 --unix_digest_hash_attribute_name=user.h`, with a made-up digest in
  the attribute of the source file: both sides log the attribute's value as the file's digest
  (`hashFunctionName: GITSHA1`); logs, cache layout (`gitsha1/...`) and `aquery` are equal.
- `--digest_function=md5`: both print `While parsing option --digest_function=md5: Not a valid
  hash function.`
- A build with `sha1`, then with `blake3` on the same output base: both run all three actions
  again, then none on a third run.

Findings:

- D-1 (S2, does not block). `crates/blaze_repo_fetch/src/cache/download_cache.rs:424-427`:
  `if ... && let Ok(Some(digest)) = path.get_fast_digest()` drops an I/O error of the attribute
  read and falls back to hashing the file. Java `DownloadCache.java:373-377` lets the
  `IOException` of `path.getFastDigest()` out. Masked: needs
  `--unix_digest_hash_attribute_name` and a failing `getxattr`. Expected fix: `?` on the error.
- D-2 (S4). `crates/blaze_runtime/src/bazel_modules.rs:393-396`: the comment still says SHA-256
  only and that the attribute option fails loudly.

Open, outside the item: on the invalid value the oracle also prints `Server crashed during
startup. Now printing .../server/jvm.out` before the message; Blaze does not. With a changed
`--digest_function` the oracle prints the `Running Bazel server needs to be killed` warning;
the Blaze binary started a new process for every command in these probes (server mode is opt-in),
so the warning could not be compared.

Not read: `Path.getDirectoryDigest` has no port (no `// Java:` marker anywhere); the remote
callers of `DigestUtil` beyond the function's wiring (earlier remote rounds cover them);
`RemoteServerCapabilities` digest function checks.

## 2. WebAssembly in repository rules: STOP (W-1, W-2)

Read: `crates/blaze_repo_rules/src/starlark/wasm.rs` (whole), `load_wasm` and `execute_wasm` in
`base_external_context.rs:2704-2826`, against `StarlarkWasmModule.java`,
`StarlarkWasmExecutionResult.java` and `StarlarkBaseExternalContext.java:2110-2320`.

Probe: one repository rule running 30 cases over four hand-assembled modules
(`wsw/repo.bzl`, modules in `wasm/*.wat`), both sides with
`--experimental_repository_ctx_execute_wasm`; full outputs in `res/wasm-oracle.txt` and
`res/wasm-blaze.txt`.

Equal on both sides: `repr` and `type` of the module and of the result, `module.path`, use as a
dict key, input and output bytes above 0x7f and NUL, the negative return code turned into
4294967295, growth within and beyond `memory_limit`, the timeout of a spinning function
(`Error executing spin: timed out`), `allocation failed: alloc_null(3, 1) returned NULL`,
`Unknown export with name nosuch`, the missing `allocate_fn` export, the out-of-range
`memory_limit`, the wrong type of `timeout`, and the repository's marker file (the same `FILE:`
lines and digests for all four modules).

Different (engine error texts for traps and out-of-bounds access are wasmi's, as the ruling of
2026-11-26 allows; these are not findings):

- W-1 (S2, blocks). `wasm.rs:295` (`wasmi::Module::new`) with wasmi's default features, which
  include `wat` (`Cargo.lock`: `wasmi 1.1.0` depends on `wat`): Blaze loads a module in text
  format. Probe: `p.wat` copied to `bad.wasm`, `load_wasm(Label("//:bad.wasm"))` succeeds in
  Blaze and the repository is created; Bazel fails the fetch (`Parser.parse`,
  `StarlarkWasmModule.java:65`: `magic header not detected, found: [...] expected: [0, 97, 115,
  109]`). The same cause makes Blaze's message for a file that is not WebAssembly a five-line
  text-parser diagnostic (`expected '('  --> <anon>:1:1 ...`). Expected fix: build wasmi without
  the `wat` feature (or reject input without the binary magic before the engine sees it).
- W-2 (S1, blocks). `wasm.rs:454-461`: the comment says wasmi keeps the module's own memory
  limits and only caps growth. Java `getMemLimits` + `withMemoryLimits`
  (`StarlarkWasmModule.java:152, 245-271`) replaces the module's limits with
  `(min(initial, limit pages), limit pages)`. Probe results:
  - `(memory 1 2)`, `memory.grow 4`, default limit: oracle `rc=1` (growth succeeds), Blaze
    `rc=4294967295` (growth refused at the module's own maximum).
  - `(memory 3)`, `memory_limit = 65536` or `0`: oracle runs with one page (`memory.size` gives
    `rc=1`), Blaze returns `rc=-1`, `failed to instantiate memory: a resource limiter denied
    ...`.
  Expected fix: the binary is already rewritten for the memory export; rewrite memory 0's limits
  there to the computed `MemoryLimits` (which also removes the unused `initial_pages`).
- W-3 (S2). `wasm.rs:487-498` and `call`: Chicory does not check a function's signature, wasmi
  does. Probe: an export `(result i32)` with no parameters called as the function gives `rc=3`
  on the oracle and `rc=-1`, `encountered an incorrect number of parameters` in Blaze; the same
  for `allocate_fn` naming such a function, and for calling `allocate` as the function (oracle
  `rc=1051`). An export that is a global: oracle `The export notfn is of type GLOBAL and cannot
  be converted to FUNCTION`, Blaze `Unknown export with name notfn` (a Chicory text used for the
  wrong case). Masked: needs a module whose function does not have the documented signature.
- W-4 (S2). `base_external_context.rs:2812` and `wasm.rs:562-585`: a negative timeout is raised
  to zero, and the deadline is only looked at when a fuel slice runs out. Probe: `timeout = -1`,
  function `echo`: oracle `rc=-1`, `Error executing echo: timed out`; Blaze `rc=1`, output
  `"q"`. Java: `invokeAny(..., timeout.toMillis(), ...)` (`StarlarkWasmModule.java:130-133`).
- W-5 (S2). `base_external_context.rs:2713-2722, 2746-2754`: with the flag off the method fails
  when called instead of not existing. Probe: oracle `repo.bzl:42:12 ... Error:
  'repository_ctx' value has no field or method 'execute_wasm'`; Blaze `repo.bzl:42:25 ...
  Error in execute_wasm: 'repository_ctx' value has no field or method 'execute_wasm'`
  (`hasattr` and `dir` differ too). Declared in the code as a limit of starlark-rust.
- W-6 (S3). `WorkspaceRuleEvent.newLoadWasmEvent` / `newExecuteWasmEvent` are not posted (marked
  `PORT:` in both methods); profiler tasks `WASM_LOAD` / `WASM_EXEC` are skipped; the call is
  not interruptible before its timeout (Java's `invokeAny` is).
- W-7 (S4). A module without a memory: oracle `Cannot invoke "...Memory.write(int, byte[])"
  because "memory" is null`, Blaze `WebAssembly module has no memory`. Both fail.

No JUnit test exists at the pin; the shell test `test_load_and_execute_wasm`
(`starlark_repository_test.sh:3352`) covers `run_ok` and `run_err` only. The two unit tests in
`wasm.rs` take their expectations from the Java arithmetic, not from Blaze.

Not read: the `wasmi` crate itself; fuel accounting against Chicory's cost (declared deviation).

## 3. `starlark_doc_extract`: PASS

Probe: `ws/doc/src.bzl` (140 lines: functions with every kind of default value, lambdas,
providers with and without `init` and field docs, six rules covering every attribute type,
test and executable rules, an initializer, three aspects, three symbolic macros with
`inherit_attrs` of `"common"` and of a rule, a repository rule, a module extension with two tag
classes, nested structs, aliases and re-exports) plus `dep.bzl`. Targets: all symbols;
`symbol_names` with a nested struct, a missing name and `render_main_repo_name`; the dependency
alone; a missing `deps` entry.

- `doc.textproto` (1,466 lines, 30 entries), `doc_filtered.textproto` (272 lines) and
  `doc_dep.textproto` (41 lines) are equal on both sides, and so are the three `.binaryproto`
  files byte for byte.
- The missing dependency gives the same error on both sides (`missing bzl_library targets for
  Starlark module(s) //doc:dep.bzl`).

Tests: the crate has 77 tests in five files, the same counts per class as the JUnit classes
(39, 3, 22, 9, 4); one is ignored with a reason (`all_native_rules_are_supported`).

- E-1 (S3). `attribute_info_extractor.rs:63, 84`: the default of a native `ComputedDefault` or
  `LateBoundDefault` whose Java class name is not recorded calls `unported!`. Reached by
  documenting a macro that inherits attributes from a native rule.

Not read line by line: `module_info_extractor.rs`, `starlark_function_info_extractor.rs`,
`label_renderer.rs`, the rule side in `blaze_rules/src/starlarkdocextract/` (the probe covers
them; no code path was found by reading that the probe did not take, apart from E-1). Not
probed: modules in other repositories, `--incompatible_...` flags that change the output.

## 4. `--proto:rule_classes`, Android and J2ObjC fragments, feature flags: PASS

Probes (outputs in `res/ff-oracle`, `res/ff-blaze`), equal on both sides in every case:

- `query --output=streamed_jsonproto --proto:rule_classes` over genrules, `config_feature_flag`,
  `config_setting` and `starlark_doc_extract` (26 KB), the same with `--noproto:default_values
  --proto:output_rule_attrs=...`, `--output=proto` (19 KB, byte equal), `--output=xml`, and
  `cquery --output=textproto --proto:rule_classes` (1,903 lines).
- `ctx.fragments.android`: `dir()` and the value of all 36 fields and methods, and
  `ctx.fragments.j2objc.translation_flags`, at the defaults and with eleven options changed
  (`--apk_signing_method=v4`, `--android_manifest_merger=legacy`, `--noincremental_dexing`,
  `--experimental_incremental_dexing_after_proguard=7`, `--j2objc_translation_flags=-Xq,-y`,
  ...). `android.get_output_directory_name` returns null in Java and crashes Bazel
  (`NullPointerException ... method invocation returned null`); Blaze panics at the same place
  (`android_configuration.rs:321`).
- Configuration checksums: `a7a71fd...` (target) and `e722a21...` (exec) at the defaults and
  `b043435` with the eleven options are the same on both sides.
- `--j2objc_translation_flags=-x,-y`: the same `J2Objc translation flags: ... not supported.
  Unsupported flags are: --prefixes,--prefix,-x` and `Build options are invalid`.
- `config_feature_flag`: `FeatureFlagInfo.value`, `is_valid_value`, `error` (flag without a
  default), a `select` on `flag_values`, and the three analysis errors (default not allowed,
  duplicate allowed values, empty allowed values) have the same text.

No finding. Not reached: the feature flag transitions (`feature_flags` attributes,
`ConfigFeatureFlagTaggedTrimmingTransition`), `--enforce_transitive_configs_for_config_feature_flag`,
`config --dump_all` (needs a kept server), a line-by-line read of the option definitions (the
checksums and the Starlark surface agree where probed).

## 5. Auto output filter and glob interrupt (skim): PASS

- Read `output_filter`, `AutoOutputFilterSlot`, `OutputFilteringHooks` and
  `auto_output_filter_get_filter` (`blaze_runtime/src/commands/build.rs:3400-3610`) against
  `AutoOutputFilter.java` and `OutputFilteringModule.java`: the regular expressions, the
  `java(tests)?` merge, the super-package skip and the precedence of `--output_filter` follow
  the Java.
- Probe: a target in `//java/foo` depending on `//javatests/foo`, `//javatests/foo/sub` and
  `//other`, each action writing to stderr, with `--auto_output_filter=none|all|packages|
  subpackages`, with `--output_filter=other` on top, and with an invalid value: the lines shown
  are the same on both sides in all six cases.
- Glob interrupt: skimmed the diff of `unix_glob.rs`, `glob_cache.rs` and `globs.rs`
  (`glob_interruptible`, `GlobberError::Interrupted`, an interrupted glob is not cached). Four
  JUnit tests that were ignored now run with injected failures. No finding; not probed (an
  interrupt during package loading is hard to time).

## Note for root

`df -h /home` showed 94 GB free during this review (below the 150 GB mark of AGENTS.md).

## Verdicts (bound to `8ecc520`)

1. Hash functions: PASS.
2. WebAssembly in repository rules: STOP. Blocking: W-1 (text-format modules are accepted),
   W-2 (the module's memory limits are kept instead of replaced by the computed limits).
3. `starlark_doc_extract`: PASS.
4. `--proto:rule_classes`, Android and J2ObjC fragments, feature flags: PASS.
5. Auto output filter and glob interrupt: PASS.
