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
