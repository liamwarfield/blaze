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
| 0005 | applied | string methods: `elems()` sequence; `split`/`rsplit` (`sep` required and nameable, named `maxsplit`, "Empty separator"); negative `replace` count |
| 0006 | applied | `list.pop` with a negative index |
| 0007 | applied | set semantics (in-place `\|=` `&=` `^=` `-=`, variadic methods, `pop` order, `set()` repr, `isdisjoint` and the `*_update` methods, Java's argument errors) |
| 0008 | applied | float repr and `%e`/`%f`/`%g` (Java's digits), `%d`/`%o`/`%x`/`%X` on floats and Java's two's complement, exact int/float comparison, "int too large to convert to float", division-by-zero texts |
| 0009 | applied | builtin `type()` / `repr` names (`builtin_function_or_method`, `<built-in function len>`, `<built-in method m of T value>`, `<function f[ from M]>`); builtins unhashable |
| 0010 | queued | error texts, only where a test or the oracle needs them |
| 0011 | applied | `%s` and `"{}".format` use `str`, not `repr` |
| 0012 | applied | per-thread heap chunk size overflow |
| 0013 | applied | per-type unknown-attribute message (`StarlarkValue::get_attr_error_message`) |
| 0014 | applied | reflected `\|` (`StarlarkValue::rbit_or`), so `dict \| select(...)` reaches the select |
| 0015 | applied | public set allocation and heap-free iteration (`set::alloc_set`, `SetRef` accessors) |
| 0016 | applied | immutable lists on a mutable heap (`list::alloc_immutable_list`, `list::list_is_immutable`) |
| 0017 | applied | a function's defining module (`eval::DefModule`, `Evaluator::innermost_enclosing_def_module`) |
| 0018 | applied | immutable dicts on a mutable heap (`dict::alloc_immutable_dict`, `dict::dict_is_immutable`) |
| 0019 | applied | `dict.get` / `dict.setdefault` accept `default` by name |
| 0020 | applied | Bazel 9.3.2 type syntax in `DialectTypes::ParseOnly` (tolerant, unresolved annotations; `cast`, `isinstance`, `type X = T`, `def f[T]`); var statements: 0030 |
| 0021 | applied | `FrozenModule::names_any_visibility` (Java `Module.getGlobals`) |
| 0022 | applied | a function's defining module's extra value (`DefModule::frozen_extra_value`) |
| 0023 | applied | mutating a frozen list or dict fails with Java's text (part of 0010) |
| 0024 | applied | a function's declaration position (`DefModule::declaration_span`, Java `StarlarkFunction.getLocation`) |
| 0025 | applied | immutable sets on a mutable heap (`set::alloc_immutable_set`, `set::set_is_immutable`); mutating a frozen set fails with Java's text |
| 0026 | applied | fallible attribute access with the evaluator (`StarlarkValue::get_attr_fallible` / `has_attr_fallible`); `getattr` default only for a missing field |
| 0027 | applied | `x[i]` and `in` with the evaluator (`StarlarkValue::at_with_eval` / `is_in_with_eval`) |
| 0028 | applied | Java error locations (callee frame for argument errors, `=` of index/unpack assignments, `.` of method lookups and `x.f op=`, `if` conditions, `for` collections, dict `:`); no call inlining; `int()` parse errors are value errors |
| 0029 | applied | every unresolved name of a file (`starlark::eval::resolve_errors`) |
| 0030 | applied | var statements `x: T` under type syntax (bound, never assigned) |
| 0031 | applied | a cycle in repr/str prints `...` |

Conformance (Bazel ScriptTest, `starlark_conformance run`; strict = Java ScriptTest semantics,
resilient = aborted chunks re-run statement by statement):

| tree | chunks | strict asserts | resilient asserts | unsafe-to-run asserts |
|---|---|---|---|---|
| stock 0.14.2 + harness json shim | 28/95 | 2060 | 2356/2714 | 4 (panic, 2 GiB+ allocations, stack overflow) |
| + patches 0001–0004, 0006 | 30/95 | 2077 | 2379/2725 | 0 |
| + blaze_starlark universe and `json` port | 30/95 | 2180 | 2443/2744 | 0 |
| + 0005, 0007–0009, 0026–0031 (tree of 2026-11-18, before commit) | 37/95 | 2494 | 2567/2773 | 0 |

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

## 0005 — string methods

- Gap: Java `str.elems()` returns `StarlarkList.wrap(null, strings)` (net/starlark/java/eval/
  StringModule.java:932), an immutable list; starlark-rust returned a lazy `iterator` value, so
  `content.elems()[:]` (rules_python `parse_requirements_txt.bzl:59`, reached from workerd's
  `pip` extension) failed with "Operation `[::]` not supported on type `iterator`".
- Fix: `types/string/methods.rs` `elems` allocates the 1-element strings into an immutable list
  (`alloc_immutable_list`, patch 0016); `iter.rs` `iterate_chars` is now unused. Elements are
  code points (decisions.md: code-point string semantics); Java yields the internal Latin-1
  chars, i.e. UTF-8 bytes, which differ only for non-ASCII strings.
- Also (second part): `split` and `rsplit` follow net/starlark/java/eval/StringModule.java:387 and
  :426: `sep` is required (Java has no whitespace split) and may be named, `maxsplit` is an int,
  positional or named, negative meaning no limit; an empty separator fails with "Empty
  separator" (both methods). `replace` (StringModule.java:338) treats a negative `count` as no
  limit (was "Replace final argument was negative"). The whitespace-split helpers are unused
  (`#[allow(dead_code)]`).
- Tests: `patch_0005_elems_list`, `patch_0005_split_and_replace`.

## 0006 — `list.pop(negative)`

- Bug: `[1, 2, 3].pop(-2)` failed "Index `-2` is out of bound" (Bazel: 2).
- Fix: `types/list/methods.rs` `pop` resolves negative indexes against the length
  (StarlarkList.pop, EvalUtils.getSequenceIndex).
- Test: `patch_0006_list_pop_negative_index`.

## 0007 — set semantics

- Gap: starlark-rust's `set` differed from Java's StarlarkSet: `s |= t`, `&=`, `^=`, `-=` rebound
  `s` to a new set (Java Eval.inplaceBinaryOp, net/starlark/java/eval/Eval.java:498-536, mutates
  the left set when both operands are sets); `union`, `intersection`, `difference`, `update`
  took exactly one argument (Java: any number, StarlarkSet.java:451-723); `isdisjoint`,
  `intersection_update`, `difference_update`, `symmetric_difference_update` were missing; `pop`
  removed the last element (Java: the first, StarlarkSet.java:570); `repr(set())` was `set([])`
  (Java `set()`, StarlarkSet.java:216); arguments were any iterable (Java
  `toHashableCollection`, StarlarkSet.java:818: a set, dict keys, or a list/tuple/range, else
  "for <method> argument got value of type 'T', want a collection of hashable elements").
- Fix: `types/set/methods.rs`: the methods above rewritten after Java (texts: "set is empty",
  "element X not found in set"; `difference_update` names "intersection_update argument" as
  Java does); `hashable_collection`, `set_inplace_op` (`SetInplaceOp`). `eval/compiler/stmt.rs`
  `bit_or_assign` and new opcodes `SubAssign`, `BitAndAssign`, `BitXorAssign`
  (`bc/opcode.rs`, `bc/instr_impl.rs`, `bc/compiler/assign_modify.rs`) mutate a set in place;
  other operands keep the plain operator. Mutation goes through `SetMut` (0025's frozen text).
  `types/set/value.rs` Display: `set()` when empty.
- Not done: error texts of operators (`unsupported binary operation`), `unhashable type`,
  argument-count texts (queued 0010); Java's shallow `freeze()` (harness).
- Test: `patch_0007_set_semantics`.

## 0008 — floats

- Gap: `str(float)` used a `%g`-like format with 6 digits (`1.23e-45` where Bazel prints
  `1.2299999999999998e-45`; `1e16` where Bazel prints `10000000000000000.0`); `%d`/`%o`/`%x`/
  `%X` rejected floats; int/float comparison went through f64 (inexact for big ints); a big int
  combined with a float became `inf` (Java: "int too large to convert to float"); division texts
  were starlark-rust's.
- Fix (Java at net/starlark/java/eval/StarlarkFloat.java:120 `format`, Printer.java:353-400
  `formatWithList`, StarlarkInt.java:376 `toFiniteDouble`, :756 `compareIntAndDouble`):
  - `types/float/float.rs`: Java's digits are the shortest round-trip digits (Formatter /
    FormattedFloatingDecimal), rounded half-up for a precision: `java_e` (`%e`), `java_f` (`%f`),
    `java_g` (`%.17g` with StarlarkFloat's trailing-zero clean-up; used by `str`, `repr`, `%g`).
    Division texts: `//` "integer division by zero", `%` "floating-point modulo by zero".
  - `types/string/interpolation.rs`: `%d %o %x %X` take an int or a finite float (truncated;
    "got nan, want a finite number"); an int32/int64 prints in two's complement for `%o %x %X`
    (java.util.Formatter on Integer/Long), larger ints as sign and magnitude; other types fail
    "got T for '%c' format, want int or float". `%e %f %g` convert ints with toFiniteDouble.
  - `types/num/value.rs`: `NumRef` equality/ordering compare an int and a float exactly
    (`cmp_int_float`); `as_finite_float`, `check_float_operands` (used by float and big-int
    `+ - * / // %`, and `float()`); `/` by zero "floating-point division by zero".
  - `types/int/int_or_big.rs`: int `//` and `%` by zero "integer division by zero" / "integer
    modulo by zero"; `int(inf)` "can't convert float +inf to int" (MethodLibrary.java:634).
- Not done: the texts of `float("x")` ("invalid float literal: x") and of a string beyond the
  double range ("floating-point number too large"), left to 0010.
- Test: `patch_0008_floats`.

## 0009 — builtin type and repr names

- Gap: `type(len)` and `type([].clear)` were "function" (Java `builtin_function_or_method`,
  net/starlark/java/eval/BuiltinFunction.java:38); `str(len)` was `len` (Java
  `<built-in function len>`), `str([].clear)` `clear` (Java `<built-in method clear of list
  value>`, BuiltinFunction.java:192); a def printed as `file.f` (Java `<function f>`, plus
  ` from <client data>` when the module has client data, StarlarkFunction.java:222); builtins
  were hashable (Java: not immutable, so "unhashable type").
- Fix: `values/types/function.rs` `BUILTIN_FUNCTION_TYPE` for `NativeFunction` and
  `BoundMethodGen`, their repr, and `name_for_call_stack` (the bare name, so tracebacks still say
  `Error in len`); `eval/compiler/def.rs` Display of `DefGen`: `<function NAME>` and, for a def
  whose defining module is frozen with a non-None extra value, ` from ` + that value's `str`.
- Users: everything that tested `get_type() == "function"` for natives (blaze_starlark json and
  structs updated; see the consumer list in docs/research/05-starlark-rust.md section 9).
- Test: `patch_0009_function_type_and_repr`.

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

## 0018 — immutable dicts

- Gap: Java `Dict.immutableCopyOf` (net/starlark/java/eval/Dict.java:495, via `copyOf(null, ..)`)
  makes a dict with the frozen `Mutability.IMMUTABLE` on any thread: every mutator calls
  Starlark.checkMutable first ("trying to mutate a frozen dict value"), and `isImmutable()` is
  `mutability().isFrozen()` (Dict.java:180), so keys and values are not looked at. Used by
  `cc_internal.freeze` for dicts.
- Fix: `types/dict/value.rs`: the mutable dict's payload is `MutableDictData { cell:
  RefCell<Dict>, immutable: Cell<bool> }` (was `RefCell<Dict>`; `Dict` itself is unchanged
  because `FrozenDictData` coerces to it). `DictMut::from_value` (`update`, `pop`, `popitem`,
  `setdefault`, `clear`, `|=`) and `set_at` (`d[k] = v`) fail with Java's text before the
  iteration check. `alloc_immutable_dict(heap, Dict) -> Value` (the empty dict is the shared
  frozen empty dict, Java `Dict.empty()`), `dict_is_immutable(value) -> Option<bool>` (frozen
  or flagged: `Some(true)`; other dicts `Some(false)`; non-dicts `None`); re-exported from
  `starlark::values::dict`. Freezing turns a flagged dict into an ordinary frozen dict; dicts
  stay unhashable. `refs.rs` drops its now unused `RefCell` import.
- Users: blaze_starlark `dict::dict_immutable_copy_of` (`cc_internal.freeze`).
- Test: `patch_0018_immutable_dict`.

## 0019 — named `default` in `dict.get` and `dict.setdefault`

- Gap: Java `Dict.get`, `Dict.pop` and `Dict.setdefault` declare `default` with `named = true`
  (net/starlark/java/eval/Dict.java:227, 260, 312), so `d.get(k, default = "")` works;
  starlark-rust made it positional-only for `get` and `setdefault` ("Found `default` extra named
  parameter(s) for call to function"). toolchains_llvm's `llvm_distributions.bzl:236` does
  `rctx.attr.sha256.get(key, default = "")` (workerd's LLVM toolchain repo).
- Fix: `types/dict/methods.rs`: drop `require = pos` from `default` in `get` and `setdefault`
  (`pop` already accepted it). `key` stays positional-only, as in Java.
- Test: `patch_0019_dict_named_default`.

## 0020 — Bazel 9.3.2 type syntax (`DialectTypes::ParseOnly`)

- Gap: `--experimental_starlark_type_syntax` defaults to true at 9.3.2
  (lib/packages/semantics/FlagConstants.java:36) and type checking to false, so
  BzlCompileFunction.java:199-214 parses every non-prelude .bzl with `allowTypeSyntax(true)`,
  `resolveTypeSyntax(false)`, `tolerateInvalidTypeExpressions(true)`: annotations are arbitrary
  expressions (net/starlark/java/syntax/Parser.java:1084 `parseTypeExprWithFallback` calls
  `parseTest`), the Resolver does not visit them (Resolver.java:874, 883, 1030) and the evaluator
  ignores them. starlark-rust's `ParseOnly` still required well-formed type expressions
  (`TypeExprUnpackP::unpack`) and resolved their identifiers (undefined names and locals were
  errors), and has no `cast`, `isinstance`, type alias or type parameter syntax.
- Fix (all only when `enable_types == ParseOnly`; `Disable` and `Enable` are unchanged):
  - `starlark_syntax/src/syntax/grammar_util.rs` `dialect_check_type`: no structural check.
  - `starlark/src/eval/compiler/scope.rs` `resolve_idents_in_type_expr`: not resolved
    (`ModuleScopeBuilder::resolve_type_exprs`).
  - `starlark/src/eval/compiler/module.rs`: annotations are not evaluated at compile time
    (`populate_types_in_stmt` only when `check_types`); `expr_for_type` already ignored them.
  - `starlark_syntax/src/syntax/parser_rd.rs`: `cast` and `isinstance` are keywords
    (Lexer.java:615): `cast(T, e[,])` parses to `e` (Parser.java:558, Eval.java:568),
    `isinstance(e, T[,])` is the resolver error "isinstance() is not yet supported"
    (Resolver.java:897), and neither is accepted as an identifier; `type X[T, ..] = T` at top
    level is a `pass` (Parser.java:1235, Resolver.java:623, Eval.java:322), inside a `def` the
    error "type alias statement not at top level" (Resolver.java:906); `def f[T, U](..)` type
    parameters are parsed and dropped (Parser.java:1255).
- A var statement (`x: T` without a value) was a "not ported" parse error here; patch 0030
  ports it. `...` stays accepted outside annotations (upstream behaviour for every
  non-`Disable` mode; Java: only inside type expressions). Doc comments (`#:`) are comments.
- Users: blaze_starlark `FileKind::dialect_with_options` / `eval::compile_with_options`
  (BZL_COMPILE's `shouldUseTypeSyntax`).
- Test: `patch_0020_bazel_type_syntax`.

## 0021 — every binding of a frozen module

- Gap: Java `Module.getGlobals()` returns all global bindings; PackageFunction.java:1068 takes
  the BUILD prelude's (`_x` names and, since the prelude is compiled with `loadBindsGlobally`,
  load-bound names included). `FrozenModule::names()` lists public names only.
- Fix: `starlark/src/environment/modules.rs` `FrozenModule::names_any_visibility()`; values are
  read with the existing `get_any_visibility`.
- Test: `patch_0021_frozen_module_names_any_visibility`.

## 0022 — the extra value of a function's defining module

- Gap: 0017's `DefModule` identifies a module by its compiled file name, but Bazel executes one
  compiled .bzl into several modules (BzlLoadFunction.java:907-918: `KeyForBuild` /
  `KeyForBzlmod` and `KeyForBzlmodBootstrap` with different repo mappings), and reads the
  module's client data from the function itself (`BazelModuleContext.of(fn.getModule())`,
  lib/cmdline/BazelModuleContext.java:163-187).
- Fix: `environment/modules.rs`: `FrozenModuleData` keeps the frozen `extra_value` (also still
  on `FrozenModule`); `eval/compiler/def.rs` `DefModule::frozen_extra_value()`. For a function
  whose module still executes the caller reads `Evaluator::module().extra_value()`.
- Users: blaze_starlark `defining_module::module_extra_value_of_function` /
  `module_extra_value_of_innermost_enclosing_starlark_function`; blaze_packages
  `BazelModuleContext::of_function` / `of_innermost_bzl`; `Label()`.
- Test: `patch_0022_def_module_frozen_extra_value`.

## 0023 — error text for mutating a frozen list or dict

- Gap: Java `Starlark.checkMutable` (net/starlark/java/eval/Starlark.java) fails with "trying to
  mutate a frozen list value" / "... dict value"; starlark-rust says "Immutable" for values on a
  frozen heap (0016 and 0018 already use Java's text for flagged-immutable values). Needed by
  PackageFunctionTest.testPreludeSymbolCannotBeMutated. This is the part of queued patch 0010
  (error texts where a test needs them) for these two types.
- Fix: `types/list/value.rs` (`ListData::from_value_mut`, `FrozenListData::set_at`),
  `types/dict/refs.rs` (`DictMut::from_value`), `types/dict/value.rs` (`FrozenDictData::set_at`).
- Test: `patch_0023_frozen_mutation_text`.

## 0024 — a function's declaration position

- Gap: Bazel names where a function is declared in two errors (net/starlark/java/eval/
  StarlarkFunction.java:172 `getLocation`): "to avoid unintended retention of analysis data
  structures, the map_each function (declared at %s) must be declared by a top-level def
  statement" (analysis/starlark/Args.java:380, TemplateDict.java:72) and the same for
  `resource_set` (StarlarkActionFactory.java:1077). The position is the name of a `def` and the
  `lambda` keyword of a lambda (oracle: `cases.bzl:9:9` for a nested `def inner`, `:21:11` for
  `_lambda = lambda x: x`; crates/blaze_actions/tests/data/map_each_probe/expected.txt).
  starlark-rust keeps only `DefInfo::signature_span`, which for a lambda is the span of its
  parameters, and exposes neither.
- Fix: `eval/compiler/def.rs`: `DefInfo::declaration_span` (the name span of a `def`, the
  expression span of a `lambda`), passed to `Compiler::function` by `compiler/stmt.rs` and
  `compiler/expr.rs`; `DefModule::declaration_span(function) -> Option<FileSpan>`.
  `signature_span` is unchanged.
- Users: blaze_actions `analysis/starlark/args.rs` (`map_each` of `Args` and `TemplateDict`).
- Test: `patch_0024_def_declaration_span`.

## 0025 — immutable sets

- Gap: Java `StarlarkSet.immutableCopyOf` (net/starlark/java/eval/StarlarkSet.java:374, via
  `copyOf(null, ..)`) makes a set with the frozen `Mutability.IMMUTABLE` on any thread: every
  mutator calls Starlark.checkMutable first ("trying to mutate a frozen set value"), and
  `isImmutable()` is `mutability().isFrozen()` (StarlarkSet.java:187), so elements are not looked
  at. Used by `Type.SetType.copyAndLiftStarlarkValue` (lib/packages/Type.java:846). starlark-rust
  sets are mutable until their heap freezes, and a set on a frozen heap said "Immutable"
  (`ValueError::CannotMutateImmutableValue`) where Java says "trying to mutate a frozen set
  value" (the same gap 0023 closed for lists and dicts).
- Fix: `types/set/value.rs`: the mutable set's payload is `MutableSetData { cell:
  RefCell<SetData>, immutable: Cell<bool> }` (was `RefCell<SetData>`; `SetData` itself is
  unchanged because `FrozenSetData` coerces to it; Debug output unchanged when the flag is
  unset). `types/set/refs.rs`: `SetMut::from_value`, the only way to a mutable set (`add`,
  `clear`, `discard`, `pop`, `remove`, `update`; the operators `|`, `&`, `^`, `-` build new
  sets, and the `*_update` methods and in-place operators are queued patch 0007), fails with
  Java's text for a flagged set, before the iteration check (Java's order), and for a set on a
  frozen heap (was "Immutable"). `alloc_immutable_set(heap, values) -> Result<Value>` (same
  arguments as 0015's `alloc_set`: hashes each value, drops duplicates, keeps first-insertion
  order, fails on unhashable values), `set_is_immutable(value) -> Option<bool>` (frozen or
  flagged: `Some(true)`; other sets `Some(false)`; non-sets `None`); re-exported from
  `starlark::values::set`. Freezing turns a flagged set into an ordinary frozen set; equality
  and hashing are unchanged. `refs.rs` drops its now unused `RefCell` import. Not done: Java's
  shared `StarlarkSet.empty()` (an empty immutable set is a flagged set on the heap; the
  vendored crate has no static empty set, and identity is not observable from Starlark).
- Users: blaze_starlark `set::set_is_immutable` (`structs::is_immutable`), blaze_packages
  `types/type_.rs` (`string_set` values lifted by `Type::copy_and_lift_starlark_value`).
- Test: `patch_0025_immutable_set`.

## 0026 — fallible attribute access with the evaluator

- Gap: `StarlarkValue::get_attr` cannot fail or see the thread, but Bazel's
  `Structure.getValue(StarlarkSemantics, String)` throws (`ctx.fragments.cpp` when undeclared:
  "err has to declare 'cpp' as a required fragment ...", RuleContext.java:512) and reads the
  semantics (`--experimental_platforms_api` fields, `enableOnlyWithFlag`). Java
  `Starlark.getattr` (net/starlark/java/eval/Starlark.java:978) returns the default of
  `getattr(x, name, default)` only when the field is missing and propagates the error;
  `Starlark.hasattr` (Starlark.java:967) propagates it too. Blaze's `getattr(.., default)`
  returned the default (oracle: scripts/w4/scenarios/ctx-errors steps 2-3 fail).
- Fix: `values/traits.rs` `get_attr_fallible(&self, Hashed<&str>, &mut Evaluator) ->
  Result<Option<Value>>` (default: `get_attr_hashed`) and `has_attr_fallible(&self, &str, &mut
  Evaluator) -> Result<bool>` (default: `has_attr`); vtable entries (`layout/vtable.rs`);
  `Value::get_attr_fallible` / `has_attr_fallible` (methods first, as `get_attr`).
  `eval/compiler/expr.rs` `get_attr_hashed_raw` / `get_attr_hashed_bind` (used by `x.f`,
  `x.f(...)` and `x.f op= y`) call it; `compile_time_getattr` folds only methods now (a field
  may fail or depend on the thread). `stdlib/funcs/other.rs` `getattr` and `hasattr` call it
  and are no longer `speculative_exec_safe`.
- Rust callers of `Value::get_attr` (no evaluator) still get `get_attr`; a type that overrides
  the fallible form keeps `get_attr` consistent for the fields it can compute without a thread.
- Test: `patch_0026_fallible_attributes`.

## 0027 — `x[i]` and `in` with the evaluator

- Gap: Java `StarlarkIndexable.Threaded` (`getIndex` / `containsKey` with the StarlarkThread)
  lets `ctx.toolchains["//x:type"]` resolve the label against the innermost .bzl on the stack
  (StarlarkToolchainContext.java:106); starlark-rust's `at` / `is_in` get only a heap.
- Fix: `values/traits.rs` `at_with_eval` (default `at`) and `is_in_with_eval` (default `is_in`),
  vtable entries, `Value::at_with_eval` / `is_in_with_eval`; `bc/instr_impl.rs` `InstrArrayIndex`
  (also `x[i] op= y`) and `InstrIn` (now its own instruction, `in` and `not in`) call them.
  Compile-time folding only touches builtin values, which do not override them.
- Test: `patch_0027_index_and_in_with_evaluator`.

## 0028 — Java error locations

- Gap: several errors were located where Java does not locate them (checked against the oracle:
  crates/blaze_starlark/tests/data/locations/*.oracle.txt, recorded by
  `scripts/starlark-locations record`; 17 of 51 cases differed): an argument or recursion error
  of a `def` was reported as a builtin error at the call (Java: inside the callee's frame, at its
  declaration: StarlarkThread.push sets the frame location to StarlarkFunction.getLocation);
  `x[i] = v` and unpacking at the target's start (Java Eval.execAssignment: the `=`); a failed
  method lookup at the call's `(` (Java Eval.evalCall → evalDot: the `.`); `x.f op= y` at the
  operator (Java: the `.` for getattr/setField); an `if` condition at the `if`
  (the optimizer gave the condition the statement's span); `for` over a non-iterable at the
  `for` (Java Eval.evalAsIterable: the collection's start); a dict literal's bad key at the key
  (Java Eval.evalDict: the entry's `:`). The optimizer inlined calls to small frozen functions,
  so their errors were located at the call site with the callee missing from the traceback.
  `int("x")` failed with the lexer's `Parser` kind, which reads as a syntax error of the file.
- Fix: `eval/compiler/def.rs` `at_declaration` (argument binding and the recursion check, moved
  after binding as in StarlarkFunction.java:565-573); `compiler/stmt.rs`
  `IrSpanned<AssignCompiledValue>::located_at` (assignment and `for` targets), `if_stmt` keeps
  the condition's span, `AssignModifyLhs::Dot` carries the `object.field` span
  (`bc/compiler/assign_modify.rs`); `bc/compiler/stmt.rs` `write_for` uses an empty span at the
  collection's start (`FrameSpan::begin_point`); `bc/compiler/call.rs` `write_call_method`: the
  instruction's span is `object.method`, the frame keeps the call span, and
  `bc/instr_impl.rs` `call_method_common` / `call_maybe_known_method_common` locate the call's own
  errors at the call; `bc/compiler/expr.rs` dict keys at `FrameSpan::dict_colon`;
  `compiler/call.rs` does not inline; `types/int/int_or_big.rs` `from_str_radix` maps the lexer
  error to a value error. blaze_starlark maps spans to Java's token (`Location::bazel_position`).
- Remaining (pinned in `traceback_locations_test.rs` KNOWN_DEVIATIONS): calling a non-callable
  value, `f(*x)` with a non-iterable `x`, `f(**x)` with a non-dict `x`.
- Tests: `patch_0028_error_locations`; blaze_starlark `traceback_locations_test` (51 oracle
  cases), `traceback_test::argument_error_is_in_the_callee_frame`,
  `native_call_has_no_builtin_toplevel_frame`.

## 0029 — every unresolved name

- Gap: Java's Resolver reports every undefined name of a file (net/starlark/java/syntax/
  Resolver.java:941 `use`, errors collected and thrown together by Program.compileFile);
  `Evaluator::eval_module` stops at the first (`ModuleScopes::check_module_err`).
- Fix: `eval.rs` `starlark::eval::resolve_errors(&AstModule, &Globals) -> Vec<Error>`: the
  resolver run alone (a scratch `MutableNames` and `FrozenHeap`), every error in source order.
  `eval_module` is unchanged.
- Users: blaze_starlark `eval::resolve` (a syntax error list, Java's compile step).
- Test: `patch_0029_resolve_errors`.

## 0030 — var statements

- Gap: under type syntax Java parses `x: T` without a value as a VarStatement
  (net/starlark/java/syntax/Parser.java:1582): it binds `x` in the enclosing block
  (Resolver.java:562 createBindings) and executing it does nothing (Eval.java:324). 0020 made it
  a "not ported" parse error.
- Fix: `starlark_syntax/src/syntax/parser_rd.rs` parses it as `if 0: x = 0` whose condition, body
  and assignment carry the identifier's span (`AstStmtP::blaze_var_statement` in `ast.rs`
  recognizes that shape; no source text can produce it), so `x` is bound in the same block and
  never assigned (reads fail "referenced before assignment" until assigned). `validate.rs`
  allows it at top level. A non-identifier target fails with Java's "type annotations must have
  a single identifier on the left-hand side" (Parser.java:1586).
  `grammar_util::BLAZE_TYPE_SYNTAX_NOT_PORTED` is no longer produced (kept: consumers name it).
- Users: blaze_starlark `syntax/resolver.rs` binds it (top-level rebinding, Java's "type
  annotation on 'x' may only appear at its first declaration").
- Test: `patch_0030_var_statement`.

## 0031 — cycle repr

- Gap: a list or dict met again while printing printed `[...]` / `{...}` (other types
  `<T...>`); Java prints `...` for every value (net/starlark/java/eval/Printer.java:191).
- Fix: `types/list/value.rs`, `types/dict/value.rs` and the default
  `StarlarkValue::collect_repr_cycle` (`values/traits.rs`).
- Test: `patch_0031_cycle_repr`.
