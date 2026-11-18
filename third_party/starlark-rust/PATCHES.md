# Blaze patch queue for starlark-rust 0.14.2

Decision: docs/decisions.md (2026-11-17, vendor starlark-rust). Rationale and the full gap list:
docs/research/05-starlark-rust.md sections 3.2 and 4.3. Numbers follow the queue in that document;
a gap in the numbering is a queued patch that is not applied yet.

Rules: one entry per patch; each changed line carries `BLAZE PATCH NNNN`; each patch has a test in
`crates/starlark_conformance/tests/patches.rs` (`patch_NNNN_*`); expected behaviour comes from
Bazel 9.3.2 (oracle or the cited Java). Keep patches small; never change public signatures except
by adding.

| # | status | summary |
|---|---|---|
| 0001 | applied | `<<` overflow and Bazel's 512-bit shift limit |
| 0002 | applied | allocation limits (repeat, concat, extend) and nesting-depth guards (repr/str/to_json) |
| 0003 | applied | raw strings keep the backslash before a quote |
| 0004 | applied | opt-in recursion check (`Evaluator::set_forbid_recursion`) |
| 0005 | queued | string methods: named `sep`/`maxsplit`, negative `replace` count, `elems()` sequence, `rsplit("")` error |
| 0006 | applied | `list.pop` with a negative index |
| 0007 | queued | set semantics (in-place augmented assignment, variadic methods, `pop` order, repr, missing methods) |
| 0008 | queued | float repr (shortest round-trip), `%` on floats, exact int/float comparison |
| 0009 | queued | builtin `type()` / `repr` names (`builtin_function_or_method`, `<built-in function len>`, `<function f>`) |
| 0010 | queued | error texts, only where a test or the oracle needs them |
| 0011 | applied | `%s` and `"{}".format` use `str`, not `repr` |
| 0012 | applied | per-thread heap chunk size overflow |
| 0013 | applied | per-type unknown-attribute message (`StarlarkValue::get_attr_error_message`) |
| 0014 | applied | reflected `\|` (`StarlarkValue::rbit_or`), so `dict \| select(...)` reaches the select |
| 0015 | applied | public set allocation and heap-free iteration (`set::alloc_set`, `SetRef` accessors) |
| 0016 | applied | immutable lists on a mutable heap (`list::alloc_immutable_list`, `list::list_is_immutable`) |
| 0017 | applied | a function's defining module (`eval::DefModule`, `Evaluator::innermost_enclosing_def_module`) |

Conformance (Bazel ScriptTest, `starlark_conformance run`; strict = Java ScriptTest semantics,
resilient = aborted chunks re-run statement by statement):

| tree | chunks | strict asserts | resilient asserts | unsafe-to-run asserts |
|---|---|---|---|---|
| stock 0.14.2 + harness json shim | 28/95 | 2060 | 2356/2714 | 4 (panic, 2 GiB+ allocations, stack overflow) |
| + patches 0001–0004, 0006 | 30/95 | 2077 | 2379/2725 | 0 |
| + blaze_starlark universe and `json` port | 30/95 | 2180 | 2443/2744 | 0 |

Known gap left by 0002: GC tracing, freezing and equality of very deep values still recurse on
the native stack; Blaze runs evaluation on threads with large stacks
(`blaze_starlark::eval::run_on_evaluation_thread`, `EVAL_STACK_SIZE`).

## 0001 — `<<` overflow and shift limit

- Bug: `InlineInt::checked_shl` used `i32::checked_shl`, which only checks the shift count, so
  `1 << 31 == -2147483648` and `2 << 31 == 0` (oracle: 2147483648, 4294967296). Shift counts up
  to 100000 were accepted.
- Fix: `starlark/src/values/types/int/inline_int.rs` `checked_shl` shifts in i64 and rejects lost
  bits (falls back to BigInt). `int_or_big.rs` `left_shift` rejects counts >= 512 with Bazel's
  "shift count too large: N" (net/starlark/java/eval/StarlarkInt.java:650 shiftLeft).
- Test: `patch_0001_left_shift_overflow`.

## 0002 — allocation limits and nesting-depth guards

- Bug: `[1, 2, 3] * (1 << 30)` panicked ("AlignedSize must not exceed u32::MAX");
  `"abc" * (1 << 30)` allocated 3 GiB and `[].extend(range(2**31 - 1))` about 16 GiB; repr,
  str and `to_json` of a 100000-deep list overflowed the native stack (uncatchable abort).
- Fix:
  - `types/list/value.rs`: `MAX_ALLOC = 1 << 30` (StarlarkList.java:89), `check_repeat_elements`
    ("excessive repeat (N * M elements)", StarlarkList.repeat / RegularTuple.repeat) used by list
    and tuple `*`; `check_add_sizes` ("excessive capacity requested (N + M elements)",
    StarlarkList.addSizesAndFailIfExcessive) used by list `+`, `extend` (when the argument's
    length is known) and `x.extend(x)`.
  - `types/string/str_type.rs`: string `*` fails with "excessive repeat (N * M characters)" when
    the result exceeds `i32::MAX` bytes (EvalUtils.repeatString).
  - `recursive_repr_or_json_guard.rs`, `layout/value.rs`: nesting deeper than 1000 renders like a
    cycle in repr/str and fails `to_json` with "nesting depth limit exceeded" (Json.java:128).
- Still required of callers: a big native stack. GC tracing, freezing and equality of deep
  values recurse; Blaze runs evaluation on threads with large stacks (blaze_starlark
  `eval::run_on_evaluation_thread`). json.star's 100000-deep list passes on such a thread.
- Tests: `patch_0002_allocation_limits`, `patch_0002_nesting_depth_guards`.

## 0003 — raw strings

- Bug: in a raw string, `\"` and `\'` dropped the backslash: `len(r"\"") == 1` (oracle: 2).
- Fix: `starlark_syntax/src/lexer.rs` `string()` always keeps the backslash in raw mode
  (net/starlark/java/syntax/Lexer.java; Python semantics).
- Test: `patch_0003_raw_string_keeps_backslash`.

## 0004 — recursion check

- Gap: starlark-rust allows recursion; Bazel forbids it ("function 'f' called recursively",
  net/starlark/java/eval/StarlarkFunction.java, StarlarkThread.isRecursiveCall).
- Fix: `Evaluator::set_forbid_recursion(bool)` (default false, so upstream behaviour is
  unchanged); `eval/compiler/def.rs` `invoke_impl` fails when a function with the same compiled
  definition (`DefInfo`, so closures of one `lambda` count as the same function, as in Java) is
  active below the top frame (`CheapCallStack::has_active_def_below_top`). Cost: one stack scan
  per `def` call, as in Java.
- Test: `patch_0004_recursion_check`.

## 0006 — `list.pop(negative)`

- Bug: `[1, 2, 3].pop(-2)` failed "Index `-2` is out of bound" (Bazel: 2).
- Fix: `types/list/methods.rs` `pop` resolves negative indexes against the length
  (StarlarkList.pop, EvalUtils.getSequenceIndex).
- Test: `patch_0006_list_pop_negative_index`.

## 0011 — `%s` and `"{}".format` use `str`, not `repr`

- Bug: `"%s" % x` and `"{}".format(x)` with a single non-string `x` called `collect_repr`, so a
  value whose `str` differs from its `repr` printed wrongly (Label: `Label("@@x//:y")` where Bazel
  prints `@@x//:y`; seen in re2's `//python:_re2.so` tags). The general multi-arg paths already
  used `str`.
- Fix: `values/types/string/dot_format.rs` `format_one` and `interpolation.rs` `PercentSFormat::Str`
  use `collect_str` (net/starlark/java/eval/Starlark.java `str`, Printer formatting).
- Test: `patch_0011_percent_s_uses_str` (blaze_starlark label test).

## 0012 — per-thread heap chunk size overflow

- Bug: `next_chunk_size` panicked (`checked_shl(..).unwrap()`) once a bump held 32+ chunks.
  Reused cached chunks can be small, so a long-lived evaluator thread reaches that count without
  growing large (seen compiling `@rules_license//licenses/spdx` during re2 `deps()`).
- Fix: `values/layout/heap/allocator/alloc/per_thread.rs` maps the overflow to the existing
  2 GiB cap.
- Test: none practical (needs 32 chunk allocations on one thread); covered by re2 `deps()`.

## 0013 — per-type unknown-attribute message

- Gap: `x.f` and `getattr(x, "f")` on a value without `f` always failed with starlark-rust's
  "Object of type `T` has no attribute `f`". Bazel asks the value
  (net/starlark/java/eval/Starlark.java:1006 `Starlark.getattr` →
  `Structure.getErrorMessageForUnknownField`), e.g. a provider instance:
  "'MyInfo' value has no field or method 'f'\nAvailable attributes: a, b"
  (lib/packages/StructImpl.java:86).
- Fix: `StarlarkValue::get_attr_error_message(&self, attribute) -> Option<String>` (default
  `None`, so other types keep starlark-rust's text); `Value::get_attr_error_message`; used by
  `eval/compiler/expr.rs` `get_attr_no_attr_error`, `Value::get_attr_error` and the `getattr`
  builtin (`stdlib/funcs/other.rs`).
- Users: blaze_starlark `StarlarkInfo` (struct and provider instances).
- Test: `patch_0013_unknown_field_message`.

## 0014 — reflected `|`

- Gap: `bit_or` had no reflected form, so `{"a": 1} | select({...})` failed in the dict's `|`
  (Bazel: net/starlark/java/eval/EvalUtils.java `binaryOp` tries the built-in cases, then
  `x.binaryOp(op, y, true)`, then `y.binaryOp(op, x, false)`; packages/SelectorList.java
  `binaryOp` handles `thisLeft = false`). `+` already has `radd`.
- Fix: `StarlarkValue::rbit_or(&self, lhs, heap) -> Option<Result>` (default `None`).
  `Value::bit_or` calls the right operand's `rbit_or` when the left operand's `bit_or` fails, and
  keeps the left's error when there is no reflected form. `bit_or_assign`
  (`eval/compiler/stmt.rs`): `dict |= y` with a non-dict `y` tries `y.rbit_or` first (Java
  Eval.inplaceBinaryOp only merges `dict |= map`), and other left operands go through
  `Value::bit_or`.
- Caveat: the reflected form is tried after any failure of the left `bit_or`; built-in `bit_or`s
  only fail on operand types, as in Java.
- Users: blaze_packages `SelectorValue` / `SelectorList`.
- Test: `patch_0014_reflected_bit_or`.

## 0015 — set allocation and iteration

- Gap: no public way to allocate a set (`SetData` is crate-private) or to read a set's elements
  without a heap (`Value::iterate`). blaze_packages needs both for `attr.string_set`-style
  values (lib/packages/Type.java:846 `SetType.copyAndLiftStarlarkValue` →
  `StarlarkSet.immutableCopyOf`, and `SetType.convert` iterating a `Set`).
- Fix: `values/types/set/refs.rs`: `alloc_set(heap, values) -> Result<Value>` (hashes each value,
  drops duplicates, keeps first-insertion order; fails on unhashable values), re-exported as
  `starlark::values::set::alloc_set`; `SetRef::from_value`, `len`, `is_empty`, `iter`.
- Test: `patch_0015_alloc_set`.

## 0016 — immutable lists

- Gap: Java `StarlarkList.immutableCopyOf` (net/starlark/java/eval/StarlarkList.java:168, via
  `copyOf(null, ..)` and `wrap`) makes an `ImmutableStarlarkList` on any thread: a `list` that
  every mutator rejects (Starlark.checkMutable: "trying to mutate a frozen list value") and whose
  `isImmutable()` is `true` without looking at the elements (ImmutableStarlarkList.java:25).
  starlark-rust lists are mutable until their heap freezes, so `cc_internal.freeze(list)` results
  failed depset's "depset elements must not be mutable values".
- Fix: `types/list/value.rs`: `ListData.immutable` flag (Debug output unchanged when unset);
  `check_can_mutate` fails with Java's text before the iteration check (Java's order), which
  covers `append`/`extend`/`insert`/`pop`/`remove`/`clear`, `x[i] = v` and `x += y`.
  `alloc_immutable_list(heap, elems)` (the empty list is the shared frozen empty list, Java
  `StarlarkList.empty()`), `list_is_immutable(value) -> Option<bool>` (frozen or flagged:
  `Some(true)`; other lists `Some(false)`; non-lists `None`); re-exported from
  `starlark::values::list`. Freezing turns a flagged list into an ordinary frozen list. Lists
  stay unhashable, as in Java ("Even a frozen list is unhashable").
- Users: blaze_starlark `list::list_immutable_copy_of` (`cc_internal.freeze`), immutability
  checks of depset elements.
- Test: `patch_0016_immutable_list`.

## 0017 — a function's defining module

- Gap: Bazel reads the module a Starlark function was defined in (net/starlark/java/eval/
  StarlarkFunction.java:195 `getModule`) and the module of the innermost Starlark function on
  the stack (Module.java:121 `ofInnermostEnclosingStarlarkFunction`, StarlarkThread.java:530),
  e.g. `BazelModuleContext.of(fn.getModule())`, `fn.getModule().getGlobal(fn.getName()) != fn`
  (CcStarlarkInternal.checkToplevel, StarlarkActionFactory.validateIsTopLevelStarlarkFunction).
  starlark-rust keeps a def's frozen module privately and exposes no frame functions.
- Fix: `eval/compiler/def.rs` `DefModule` (re-exported as `starlark::eval::DefModule`):
  `of_function(value)` for `def`/`lambda` values, `def_name(value)` (Java `getName`, "lambda"
  for lambdas), `codemap()`, `file_name()` (the compiled file name; identity: Bazel executes each
  compiled file once into one module), `is_frozen()`, `same_module(..)`, `get_global(name,
  eval)` (Java `Module.getGlobal`: a frozen module is read directly, a module still executing
  only through the evaluator executing its top level). `Evaluator::innermost_enclosing_def_module
  (depth)` walks the call stack from the top counting `def`s/`lambda`s, functions the optimizer
  inlined into a call site (from the frame span's inlined frames; `inlined_frame.rs`
  `to_inlined_frames` made `pub(crate)`, `cheap_call_stack.rs` `top_nth_frame_opt`), and the
  bottom frame while `eval_module` runs (Java's toplevel function); natives are skipped.
- Deviation: starlark-rust keeps `load` bindings among a module's names, so `get_global` finds
  loaded names (Java: file-local). For "is `fn` its own module's global `fn.getName()`" this
  cannot change the answer, since a loaded value was defined elsewhere.
- Users: blaze_starlark `defining_module` (module keys for blaze_packages
  `BazelModuleContext::of_filename`).
- Test: `patch_0017_def_module`.
