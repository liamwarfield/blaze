# Review: interpreter patches 0066-0068 and the in-process multi-command fixture

Date 2026-11-26. Commit under review: 8ecc520. First review of both parts. Reviewer read-only;
no Cargo, no tests run.

**No probe was run.** The suite unit `root-suite-5f68157` stayed `active` for the whole review,
so no bazel or blaze process was started. Every finding below comes from reading the Rust at
8ecc520 against the pinned Java. Each finding names the probe that confirms it on the oracle.

## Verdicts

- **Part 1 (patches 0066, 0067, 0068): STOP**, bound to 8ecc520. Blocking: P1-1, P1-2 (both in
  0068; 0066 and 0067 have no finding).
- **Part 2 (fixture, ConfigCommandTest, DumpCommandTest, `dump` over a kept graph): PASS**,
  bound to 8ecc520.

## Scope

Read:
- `third_party/starlark-rust`: every line marked `BLAZE PATCH 0066`, `0067`, `0068`
  (`eval/compiler/def.rs`, `eval/runtime/{cheap_call_stack,inlined_frame,arguments,evaluator}.rs`,
  `eval/bc/instr_impl.rs`, `eval/compiler/stmt.rs`, `values/{traits,layout/value,layout/vtable}.rs`,
  `values/types/dict/{globals,value}.rs`, `values/types/set/methods.rs`,
  `starlark_syntax/src/frame.rs`), the three PATCHES.md entries, the three tests in
  `crates/starlark_conformance/tests/patches.rs`.
- Users: `blaze_starlark/src/error.rs` `EvalError::from_starlark`,
  `blaze_packages/src/starlark_defs/provider.rs` (`callable_location`),
  `blaze_packages/src/build/starlark_native_module.rs` (both views), `blaze_starlark/src/json.rs`
  (dict branch), `blaze_packages/src/types/type_.rs` (dict conversion, three call sites by grep).
- Java: `StarlarkThread.push`, `EvalException.formatCallStack`, `StarlarkCallable.getLocation`
  and its overrides, `StarlarkFunction.repr`, `Dict.update/equals/cast`, `Eval.evalCall` (`**`),
  `Eval.inplaceBinaryOp`, `EvalUtils.binaryOp` (PIPE), `StarlarkSet.toHashableCollection`,
  `Starlark.len/type/classType`, `Json.Encoder.encode`, `Type.DictType.convert`,
  `StarlarkNativeModule.DictLikeView/ExistingRuleView/ExistingRulesView`.
- Part 2: `crates/blaze_buildtool_testing/src/lib.rs` (whole), `crates/blaze_runtime/tests/
  {config_command_test,dump_command_test}.rs` (whole) against the Java tests (whole),
  `BuildIntegrationTestCase.java` (`createFilesAndMocks`, `getRuntimeBuilder`, `setupOptions`,
  `buildTarget`, event helpers), `TestConstants`, `commands/dump.rs` `exec`/`dump` and
  `sorted_by_count_desc` against `DumpCommand.exec`, the diff of `blaze_sky_top/src/incremental.rs`.

Not read: the other users of the fixture (`convenience_symlink_test`, `progress_reporting_test`,
`rewinding_test`, `local_diff_awareness_test`, `kept_graph_unshared_test` beyond its header),
`commands/config.rs`, the individual dump printers (`blaze_skyframe::dump`), the macro function's
own traceback code (`macro_function.rs`, older than 0066), `BlazeRuntimeWrapper` beyond its
option handling, the module list of `bazel_modules()` against `getRuntimeBuilder`.

## Part 1

### 0066 (native callable location): no finding

`Frame::callable_location` is filled from the callee for real frames and `None` for inlined ones.
`EvalError::from_starlark` puts a native callee's frame at that location both as the leaf
(`Error: ...` instead of `Error in NAME: ...`, Java `formatCallStack` lines 205-212) and as a
caller of Starlark code (Java `fr.loc = fn.getLocation()`, StarlarkThread.java:295, never updated
for a native). Java's callables with a location at the pin are `StarlarkProvider`, its
`RawConstructor` and `MacroFunction`; the first two use the patch, the macro has its own older
code (not read). Patch is minimal, marked, recorded, in `blaze.diff`.

### 0067 (function repr while the module executes): no finding

`Def::new` takes the frozen module of the creating function when there is one, else the `str` of
the executing module's extra value; `Display` prefers the frozen module. Lambdas, nested defs and
functions created through a callback from a frozen module all take the right module. Built-in
function repr is untouched. Minimal, marked, recorded.

### 0068 (Map views)

The seven places the patch names match Java (`dict(m)`, `d.update(m)`, `f(**m)`, `d | m`,
`m | d`, `m | m`, `d |= m`, set methods; entry order from `entrySet()`; `set | m` and `m | 1`
still fail). The five call instructions are all the instructions that can carry `**`. Hashing
and `view == x` are identity, as in Java. Views are snapshots, so mutation during iteration
cannot be seen, as in Java. Two more places where Java accepts any `Map` were missed:

**P1-1 (S1, blocking). `dict == view` is always false; Java compares contents.**
- Rust: `third_party/starlark-rust/starlark/src/values/types/dict/value.rs:634` `equals`:
  `DictRef::from_value(other)` is `None` for a view, result `false`.
- Java: `net/starlark/java/eval/Dict.java:209` `equals` is `contents.equals(o)`
  (`AbstractMap.equals`): any `Map` of the same size whose `get(k)` equals every value. The view
  is a `Map` (`StarlarkNativeModule.java:147`).
- Input (in a macro called from a BUILD file after a rule `x`):
  `r = native.existing_rule("x"); print(dict(r) == r, r == dict(r))`. Java by reading:
  `True False`. Blaze: `False False`. Same root for `!=` and for list or tuple equality with a
  dict element on the left.
- Fix: in `equals`, when `other` has `java_map_entries`, compare as `AbstractMap.equals` does
  (size, then each own key against the map's value). Add the case to `patch_0068_java_map_values`
  and an oracle case.

**P1-2 (S1, blocking). `json.encode(view)` gives an array of keys; Java gives an object.**
- Rust: `crates/blaze_starlark/src/json.rs:110` takes the object branch only for `DictRef`; a
  view falls to the iterable branch at line 200.
- Java: `net/starlark/java/lib/json/Json.java:171` `if (x instanceof Map<?, ?> m)`: sorted keys,
  `m.get(key)` values, errors as `in Map key "k": ...`.
- Input: `print(json.encode(native.existing_rule("x")))` for a rule without `select` values.
  Java by reading: `{"kind":"...","name":"x",...}`. Blaze: `["name","kind",...]`. The same
  branch is reached for a view nested in a list or struct.
- Fix: use `java_map_entries` in the object branch (type name in errors is `Starlark.type`,
  "Map").

**P1-3 (S2, not blocking). Dict-typed attribute conversion rejects a view.**
- Rust: `crates/blaze_packages/src/types/type_.rs:381`, `:911`, `:1006` require `DictRef`.
- Java: `packages/Type.java:567` and `:585` accept any `Map` and convert its entries.
- Masked: a view's values (strings, lists, views) almost never fit a dict attribute's value type,
  so both sides fail; the error text differs (Java names the dict element, Blaze the whole
  value). Fix with P1-2, same helper.

S4: the test value `JavaMap` in `patches.rs` declares type `"dict"`; Java's is `"Map"`.
`creation_module_extra` formats the extra value at every `def`/`lambda` creation (cost only).

Recording: the three entries are in PATCHES.md with gap, fix and test; all patch lines are
marked; `patches/blaze.diff` holds them. `KNOWN_DIFFERENCES` and `crates/starlark_conformance/src`
did not change between c051ead and 8ecc520. The three patch tests use synthetic values; their
expected strings follow the Java rules above, and PATCHES.md cites oracle cases for 0066 and for
`dict(existing_rule)`. I did not see an expectation taken from Blaze.

## Part 2

No blocking finding.

- Options: `setup_options` equals Java `setupOptions` plus `PRODUCT_SPECIFIC_FLAGS` and
  `PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS`, minus `--override_repository=bazel_tools=...`, plus
  the repository cache and `--color=no --curses=no`; each difference is documented in the file.
- Commands: the two ported tests call the dispatcher in Java too, with the same argument order
  (`build`, fixture options, target, `--nobuild`, extra args; `config --output=json ...`; `dump
  ...` without fixture options). One runtime, one dispatcher, one output base for the life of a
  fixture; the kept graph is the only state the tests rely on between commands.
- ConfigCommandTest: 11 Java tests, 11 Rust tests, same names, same assertions (Gson classes
  replaced by JSON field reads). `analyze` additionally asserts the build succeeded (stricter).
- DumpCommandTest: 2 of 2, same assertions. Neither file has an `#[ignore]`.
- `dump`: `exec`/`dump` follow `DumpCommand.exec` (order of sections, blank lines, failure
  codes, the warning on stderr). `--memory` and the two Skyfocus modes call `unported!`.
- The 14 missing edges in `--skyframe=summary`: `multi_option_smoke` asserts only "Node count",
  as Java does; neither the fixture nor a test hides the count.

**P2-1 (S2, not blocking). A loud failure became a chosen order.**
`crates/blaze_runtime/src/commands/dump.rs:449` `sorted_by_count_desc`: rule or aspect classes
with the same count in the same `HashMap` bin used to call `unported!`; now they are sorted by
key. Java's order there is the insertion order from the graph's `ConcurrentHashMap`
(`SequencedSkyframeExecutor.getSkyframeStats`), which depends on SkyKey hash codes and, as far as
I can tell, is not stable across JVM runs. The comment says so honestly and the module already
sorts the other hash-ordered dumps. I do not block on it, but there is no entry in
`docs/decisions.md`; root should record the ruling.

**P2-2 (S4).** `EventCollector` keeps every event kind; Java's keeps errors, warnings and info
(`EventCollectionApparatus`). `assert_contains_event` filters to those kinds,
`assert_does_not_contain_event` and `events()` do not (stricter than Java, never weaker).

Open items, not blocking:
- `crates/blaze_bep/tests/metrics_collector_test.rs` has about twenty tests ignored with "needs
  BuildIntegrationTestCase"; the fixture exists since checkpoint 45, so the reason is out of date.
- The fixture ports only the helpers its tests use; the rest of `BuildIntegrationTestCase`
  (`getConfiguredTarget`, `info`, `clean`, bug reporter, file system hooks) is absent, as the
  module documentation says.

## Probes to run when the suite is over (not run)

One workspace per side, a `.bzl` macro called after a `filegroup(name = "x", srcs = [])`:
`r = native.existing_rule("x")`, then `print(dict(r) == r, r == dict(r))` and
`print(json.encode(r))`; compare the oracle with `blaze-8ecc520`.
