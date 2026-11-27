# Response to the review of 2026-11-27 (small ports), item 2 and D-1/D-2

Tree: `dafaec48` plus uncommitted changes (listed per item). Binary built from that tree
(`scripts/cargo-capped build -p blaze --bin blaze`, copied to
`~/.cache/blaze/work/sweep-2/bin/blaze`).

Probe: the reviewer's `probew.sh` and `wsw/` copied to `~/.cache/blaze/work/sweep-2/wasmrev/`
(paths changed, nothing else), with four more cases: `wat` (the text-format module `p.wat` saved
as `pwat.wasm`), `badver` (the magic followed by version 2), `zerot` (`timeout = 0`) and `dir`
(`hasattr`, `dir` and `type` of `repository_ctx`, with and without the flag). Outputs:
`wasmrev/wasm-oracle.txt` and `wasmrev/wasm-blaze.txt`.

`diff wasm-oracle.txt wasm-blaze.txt` now shows only these lines:

- `trap`, `badout`, `growneg`, `allocneg`: the engine's own trap and out-of-bounds texts (Chicory
  `Trapped on unreachable instruction` / `out of bounds memory access: attempted to access
  address: ...`, wasmi `wasm \`unreachable\` instruction executed` / `out of bounds memory
  access`), which the ruling of 2026-11-26 allows. `growneg` was a different failure before
  (instantiation refused); it is now the same out-of-bounds write as on the oracle.
- `currently loading: @@+repo+r_ok//`: a progress line that depends on timing.

## W-1 (S2): text-format modules

What changed:

- `Cargo.toml:19`: `wasmi = { version = "1.1.0", default-features = false, features = ["std"] }`.
  `Cargo.lock` no longer has `wat` among wasmi's dependencies.
- `crates/blaze_repo_rules/src/starlark/wasm.rs:329` `check_header`: Chicory's own magic and
  version checks run before the engine sees the bytes, with Chicory's text. `Parser.parse`
  rethrows a `MalformedException` with the prefix `section size mismatch, unexpected end of
  section or function, `; the magic check reports `magic header not detected, found: <4 bytes>
  expected: [0, 97, 115, 109]`, the version check `unknown binary version, found: <4 bytes>
  expected: [1, 0, 0, 0]`, a short file `length out of bounds` (`Encoding.readBytes`). Bytes
  print as Java's `Arrays.toString(byte[])` (signed). The texts were read from the Chicory
  classes in the oracle's `A-server.jar` (javap) and checked against the oracle.
- Unit test `header_errors_have_chicorys_text` (expectations from the oracle output below).

Probe, both sides the same:

```
#### bad
Error in load_wasm: section size mismatch, unexpected end of section or function, magic header not detected, found: [110, 111, 116, 32] expected: [0, 97, 115, 109]
#### wat
Error in load_wasm: section size mismatch, unexpected end of section or function, magic header not detected, found: [40, 109, 111, 100] expected: [0, 97, 115, 109]
#### badver
Error in load_wasm: section size mismatch, unexpected end of section or function, unknown binary version, found: [2, 0, 0, 0] expected: [1, 0, 0, 0]
```

## W-2 (S1): memory limits

What changed (`wasm.rs`):

- The module is kept as loaded (`WasmModuleData.module_content`). Each execution compiles it with
  memory 0's limits replaced by the computed `MemoryLimits(initialPages, maximumPages)`
  (`with_memory_limits`, `wasm.rs:884`, called at `wasm.rs:506`), as Chicory's
  `Instance.builder(wasmModule).withMemoryLimits(memLimits)` builds the memory from those limits
  and not from the module's. The store limiter is gone (the compiled module carries the
  maximum).
- Unit test `memory_limits_replace_the_modules_own`.

Probe, both sides the same:

```
max2: rc=1 out="" err=""
mem3: rc=3 out="" err=""
mem3lim1: rc=1 out="" err=""
mem3lim0: rc=1 out="" err=""
```

## W-3 (S2): signatures and export kinds

What changed (`wasm.rs`):

- `export` (`wasm.rs:614`): Chicory's `Instance.Exports.function` / `getExport`: `Unknown export
  with name <n>` for a missing export, `The export <n> is of type <GLOBAL|TABLE|MEMORY> and cannot
  be converted to FUNCTION` for another kind. Used for `_initialize`, `allocate_fn` and the
  function.
- `apply` (`wasm.rs:642`): calls untyped as Chicory does (`InterpreterMachine.call`,
  `StackFrame.<init>`): the locals are `Arrays.copyOf(args, ...)`, so missing arguments are zero
  and extra ones are dropped; the result is `null` for a function without results. wasmi itself
  checks a call's arguments, so `apply` builds the arguments of the callee's own type from the
  longs. A parameter or result of a type other than i32/i64 calls `unported!` (Chicory's long
  encoding of floats, vectors and references is not ported).
- A function without results: `execResult[0]` / `allocResult[0]` on null gives the
  NullPointerException's helpful message (`Cannot load from long array because "execResult" is
  null`), following the oracle's message style of W-7 (local variable names; not probed: no
  module in the probe has such a function).

Probe, both sides the same:

```
wrongsig: rc=3 out="" err=""
notfn: rc=-1 out="" err="Error executing notfn: The export notfn is of type GLOBAL and cannot be converted to FUNCTION"
allocate: rc=1051 out="" err=""
allocnotfn: rc=-1 out="" err="Error executing echo: The export notfn is of type GLOBAL and cannot be converted to FUNCTION"
allocwrongsig: rc=3 out="\x03\x00\x00" err=""
```

## W-4 (S2): zero or negative timeout

What changed:

- `base_external_context.rs:2979`: the timeout in milliseconds (`Math.round(timeoutSeconds *
  1000L * timeoutScaling)`) is passed as is, no longer raised to zero.
- `wasm.rs:455`: a timeout of zero or less times out at once, as `invokeAny(..., timeoutMillis,
  MILLISECONDS)` throws `TimeoutException` when it has no time to wait.

Probe, both sides the same:

```
negt: rc=-1 out="" err="Error executing echo: timed out"
zerot: rc=-1 out="" err="Error executing echo: timed out"
```

## W-5 (S2): the methods exist only with the flag

What changed:

- `base_external_context.rs:2891` `wasm_methods`: `load_wasm` and `execute_wasm` moved out of
  `base_external_context_methods`; the call-time flag checks and `no_such_method` are removed.
- `context.rs:227` `RepositoryCtxWithWasm` and `context.rs:330` `ModuleCtxWithWasm`: the context
  types with the two methods (same `type`, `repr`, fields and other methods). starlark-rust's
  method table is per type and cannot depend on the thread's semantics, so the flag picks the
  type when the context is allocated (`alloc_repository_ctx`, `context.rs:252`, used at
  `repository_fetch_function.rs:1054`; `alloc_module_ctx`, `context.rs:366`, used at
  `runnable_extension.rs:521`). Methods that took `&RepositoryCtx` / `&ModuleCtx` now take the
  value and use `repository_ctx_of` / `module_ctx_of`.

Probe, both sides the same:

```
#### dir (flag on)
dir: True True True repository_ctx
#### flagoff-dir
dir: False False False repository_ctx
#### flagoff
ERROR: .../wsw/repo.bzl:42:12: An error occurred during the fetch of repository '+repo+r_tmo':
Error: 'repository_ctx' value has no field or method 'execute_wasm'
```

Not probed: `module_ctx` (same mechanism).

## W-7 (S4), done with W-3

`wasm.rs:551`: a module without a memory fails at `memory.write` with the oracle's text. Probe,
both sides the same:

```
nomem: rc=-1 out="" err="Error executing f: Cannot invoke \"com.dylibso.chicory.runtime.Memory.write(int, byte[])\" because \"memory\" is null"
```

## W-6 (S3): open

The `WorkspaceRuleEvent` log events for `load_wasm` / `execute_wasm`, the profiler tasks, and
interrupting a call before its timeout are still not ported (marked `PORT:` in both methods).

## D-1 (S2): fast digest I/O error

`crates/blaze_repo_fetch/src/cache/download_cache.rs:425`: the I/O error of
`path.getFastDigest()` propagates (as `IOException` in Java) instead of falling back to reading
the file. Not probed (needs a file system whose fast digest fails).

## D-2 (S4): stale comment

`crates/blaze_runtime/src/bazel_modules.rs:393`: the comment now says the Unix file system uses
the hash function of `--digest_function` and `--unix_digest_hash_attribute_name`.

## E-1 (S3): not changed

`attribute_info_extractor.rs:63, 84` still calls `unported!` for a native computed or late-bound
default without a recorded Java class name.

## Tests

`scripts/cargo-capped test -p blaze_repo_rules -p blaze_repo_fetch`: 307 passed, 1 failed,
8 ignored. The failure is `oracle_extraction_test::platforms_tarball_from_shared_cache_matches_oracle_extraction`:
it compares a `platforms` tarball from the shared repository cache with an oracle extraction that
now holds a newer `platforms` (different `BUILD` contents and mtimes). It does not touch the
changed code; the test's inputs changed under it.
