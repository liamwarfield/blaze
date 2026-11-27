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
| 0010 | superseded | error texts, only where a test or the oracle needs them (now ported in general: 0049–0055, docs/decisions.md 2026-11-19) |
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
| 0032 | applied | string literal escapes as Bazel's lexer: octal `\0`-`\377` one char, `\x` `\u` `\U` `\N` and unknown escapes are errors (Java's texts and positions), carriage returns kept |
| 0033 | applied | `upper`/`lower` ASCII only; `capitalize`/`title` with Java's `Character` per char; `is*` predicates with Java's matchers |
| 0034 | applied | the byte-strings semantics flag on the evaluator (`Evaluator::set_utf8_byte_strings`, default true); `strip`/`lstrip`/`rstrip` whitespace by the flag |
| 0035 | applied | string repr escapes only `"`, `\` and chars below 0x20 (`\xNN`) |
| 0036 | applied | the string repeat limit counts chars |
| 0037 | applied | lexer whitespace: tabs and carriage returns between tokens ignored; a tab anywhere in indentation (blank and comment lines, end of file included) is an error with Java's text; a comment runs to the line feed |
| 0038 | applied | number literals: no binary literals, Java's "invalid octal literal" for a leading 0, "floating-point literal too large" |
| 0039 | applied | `async` and `await` are identifiers |
| 0040 | applied | `Dialect::enable_bytes_literals` (Bazel: none; `b"x"` is the identifier `b` and a string) |
| 0041 | applied | `start`/`end` of `find`, `rfind`, `index`, `rindex`, `count`, `startswith`, `endswith` clamped as Java's `substringIndices` |
| 0042 | applied | string slices accept `None` bounds |
| 0043 | applied | no `str.codepoints` |
| 0044 | applied | `int(x[, base])`: `x` required, base 10 by default, Java's `StarlarkInt.parse` (no `_`, no base inference from a leading 0) and texts |
| 0045 | applied | `str.format` fields as Java's FormatParser: no `!r`/`!s` conversions, Java's numeric-field rule; f-strings apply `repr` themselves |
| 0046 | applied | `strip`/`lstrip`/`rstrip` accept `chars = None` |
| 0047 | applied | under type syntax `...` only inside a type annotation |
| 0048 | applied | under type syntax `cast(...)` / `isinstance(...)` in call arguments are cast/isinstance expressions; neither is a load name |
| 0049 | applied | Bazel's spelling suggestions: Java `SpellChecker` and `java.util.HashSet` iteration order (`errors::spell_checker`) |
| 0050 | applied | name resolution texts: "name 'x' is not defined" with Java's suggestion; "local/global variable 'x' is referenced before assignment." |
| 0051 | applied | value and builtin error texts (operators, comparison, indexing, slicing, unpacking, hashing, attributes, mutation during iteration, `%`, `str.format`, `float()`, `range()`, list/dict/str/set methods) as Java's; Java's semantics where the text depends on it (`list += list` only, sequence comparison tests equality first, `range` sizes in 64 bits, shift counts) |
| 0052 | applied | call argument errors in Java's order and wording (`StarlarkFunction` / `BuiltinFunction`), parameter type errors, Java's parameter names for the core builtins |
| 0053 | applied | lexer texts and positions (invalid character, indentation error, forbidden keywords, malformed number literals) |
| 0054 | applied | parser texts: "syntax error at 'TOKEN': expected KIND" and Java's context messages (list/dict/comprehension, call arguments, load, conditional, generator, implicit concatenation) |
| 0055 | applied | static-check texts as Java's Resolver/Parser (call argument order, parameter order, top-level `for`/`if`, `break`/`continue`/`return`, `load` placement, `...`, type annotations, assignment targets, trailing comma) |
| 0057 | applied | `load()` of a symbol the file does not bind globally: Java's "file 'M' does not contain symbol 'S'" with SpellChecker's suggestion over the file's globals, at the original name; `FrozenModule::java_globals` (Java `Module.getGlobals`) |
| 0058 | applied | the "want" part of builtin parameter type errors from Bazel's `@Param` annotations (`values/java_param_types.tsv`, generated from the oracle's server jar), by receiver, method and parameter |
| 0059 | applied | a builtin function has no `[]` (`list[int]` is Java's "type 'builtin_function_or_method' has no operator [](...)") |
| 0060 | applied | the nesting guard of comparisons is 20000 levels in debug and release builds (Java has none) |
| 0061 | applied | Java's call-site error locations: an argument-binding error of a call with named or `*`/`**` arguments at the enclosing statement's location (`=`, `for`, `:`); `*x`/`**x` errors and calls of a non-callable value in the caller, without the callee's frame |
| 0062 | applied | a builtin's argument checks in Java's order: parameter types as the arguments are bound (positional, then named in call order) before the arity |
| 0063 | applied | key-tracking immutable dicts (Java `Dict.ImmutableKeyTrackingDict`, `ctx.var` with `--include_config_fragments_provider`): `alloc_immutable_key_tracking_dict`, `dict_accessed_keys` |
| 0064 | applied | the running evaluator's `extra` for code without an evaluator (`eval::with_current_extra`), so a value's `str` can read the thread's semantics (Java `Starlark.str(x, semantics)`) |

Conformance (Bazel ScriptTest, `starlark_conformance run`; strict = Java ScriptTest semantics,
resilient = aborted chunks re-run statement by statement):

| tree | chunks | strict asserts | resilient asserts | unsafe-to-run asserts |
|---|---|---|---|---|
| stock 0.14.2 + harness json shim | 28/95 | 2060 | 2356/2714 | 4 (panic, 2 GiB+ allocations, stack overflow) |
| + patches 0001–0004, 0006 | 30/95 | 2077 | 2379/2725 | 0 |
| + blaze_starlark universe and `json` port | 30/95 | 2180 | 2443/2744 | 0 |
| + 0005, 0007–0009, 0026–0031 (tree of 2026-11-18, before commit) | 37/95 | 2494 | 2567/2773 | 0 |

Both of Bazel's ScriptTest targets (`starlark_conformance run` and `run --latin1`; columns as
above). `ScriptTest_Latin1` (ScriptTest.java:244-259: the chunk's UTF-8 bytes decoded one char per
byte, `-internal_bazel_only_utf_8_byte_strings` on, `_utf8_byte_strings = True`) is how Blaze
reads every Starlark file since the byte-strings switch (docs/research/07-byte-strings-rules.md),
so it is the column that matters; `ScriptTest` runs with the flag off.

| tree | mode | chunks | strict asserts | resilient asserts | unsafe-to-run asserts |
|---|---|---|---|---|---|
| through 0031 (working tree of 2026-11-18, `e98c83c` + uncommitted) | ScriptTest | 37/95 | 2494 | 2567/2773 | 0 |
| through 0031 (same) | ScriptTest_Latin1 | 37/95 | 2487 | 2561/2773 | 0 |
| + 0032–0036, blaze_starlark `json.decode` bytes (same tree + this work) | ScriptTest | 37/95 | 2494 | 2567/2773 | 0 |
| + 0032–0036, blaze_starlark `json.decode` bytes (same) | ScriptTest_Latin1 | 38/95 | 2495 | 2569/2773 | 0 |
| + 0037–0048, probe harness `fail` (working tree of 2026-11-19 on `3ab3b35`, other agents' uncommitted changes included) | ScriptTest | 37/95 | 2515 | 2588/2773 | 0 |
| + 0037–0048 (same) | ScriptTest_Latin1 | 38/95 | 2516 | 2590/2773 | 0 |
| + 0049–0055 (working tree of 2026-11-19 on `4f92912`, other agents' uncommitted changes included) | ScriptTest | 88/95 | 2608 | 2753/2777 | 0 |
| + 0049–0055 (same) | ScriptTest_Latin1 | 89/95 | 2682 | 2756/2778 | 0 |

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

## 0032 — string literal escapes

- Gap: Bazel's lexer (net/starlark/java/syntax/Lexer.java:322-404 `escapedStringLiteral`) reads
  `\0`..`\377` as one char 0..=0xFF ("octal escape sequence out of range (maximum is \377)" at
  the last digit above that) and rejects `\x`, `\u`, `\U`, `\N` and every other unknown escape
  ("invalid escape sequence: \q. Use '\\' to insert '\'.", at the char after the backslash,
  :402). starlark-rust accepted `\x41`, `\u0041`, `\U...`, octal escapes up to `\777`, and kept
  the backslash of an unknown escape (`"\q"` was two chars). It also dropped carriage returns
  inside string literals (Java keeps them), rejected a backslash before a lone CR (Java: line
  continuation), and read a raw literal's backslash-CRLF as backslash, CR, LF (an error in a
  single-quoted literal; Java: backslash-LF; a backslash-CR alone stays only on Java's quick scan
  of a single-quoted raw literal without backslash-CRLF, :463-474). An unclosed literal said "Parse error: unfinished string literal"
  (Java: "unclosed string literal", :301, at the opening quote). Non-ASCII chars of a literal
  (Latin-1 decoded source) were and are passed through unchanged.
- Fix: `starlark_syntax/src/lexer.rs`: `escape_bazel` (Java's switch, used by `string()` for
  non-raw literals; `escape` stays for f-strings, which Bazel's dialect disables),
  `BazelEscapeError`, `LexemeError::BlazeInvalidEscape` / `BlazeOctalOutOfRange` (Java's texts;
  spans at Java's positions), the raw-literal CR handling (`raw_literal_has_backslash_crlf`), CR
  kept in the slow path, `UnfinishedStringLiteral` text.
- Limitation: Java reports every lexer error of a file; starlark-rust stops at the first.
- Users: every BUILD/.bzl file; blaze_starlark `syntax::resolver` keeps the
  `stringLiteralsAreAsciiOnly` check (.scl).
- Tests: `patch_0032_lexer_escapes`; `java_recorded_latin1` (tests/data/escape_*.star against
  Bazel's interpreter, recorded with crates/starlark_conformance/java/StarlarkProbe.java);
  dialect `bazel_string_escapes`; blaze_starlark `traceback_test::non_ascii_columns_and_source_lines`
  (oracle).

## 0033 — case mapping and character predicates

- Gap: Bazel's string methods (net/starlark/java/eval/StringModule.java) are defined on the
  UTF-16 chars of the string, which in Blaze's internal strings are the bytes of the UTF-8:
  `upper`/`lower` are Guava `Ascii.toUpperCase/toLowerCase` (:174, :182); `capitalize` (:526) is
  `Character.toUpperCase(first) + Ascii.toLowerCase(rest)`; `title` (:542) uses
  `Character.isLetter/isUpperCase/isLowerCase/toLowerCase/toUpperCase` per char; `isalpha`,
  `isalnum`, `isdigit`, `islower`, `isupper`, `istitle` use the matchers `[a-zA-Z]`, javaDigit,
  `[a-z]`, `[A-Z]` (:757-878); `isspace` Guava `CharMatcher.whitespace()`. starlark-rust used
  Rust's Unicode case mapping and properties (`"aé".upper()` changed the bytes of `é`,
  `"aé".islower()` differed; oracle values in docs/research/07-byte-strings.md 2.3).
- Fix: `values/types/string/methods.rs`: the methods above follow Java; helpers `java_is_letter`,
  `java_is_upper_case`, `java_is_lower_case`, `java_to_upper_case`, `java_to_lower_case`,
  `java_is_digit`, `guava_is_whitespace`, `java_matches` (StringModule.matches). For
  U+0000..U+00FF (every char of an internal string) they are Java's `Character` table, checked
  against the JDK of Bazel's install base: `ÿ` U+00FF upper-cases to U+0178 and `µ` U+00B5 to
  U+039C, leaving Latin-1 as in Java; `ß` U+00DF stays `ß` (no one-char upper case); `ª`, `º`,
  `µ`, `ß` are lower case. Above U+00FF (not reachable from Blaze) the nearest Rust property
  stands in, and chars above U+FFFF (a surrogate pair in Java) are uncased non-letters.
- Tests: `patch_0033_case_mapping_and_predicates`; `java_recorded_latin1` (tests/data/
  latin1_chars.star: every Latin-1 char through 31 operations, against Bazel's interpreter);
  starlark_conformance `byte_strings::byte_string_cases_match_oracle` (oracle cases
  `upper_zh`, `lower_E`, `islower_ae`, `capitalize_zh`, `title_ete`).

## 0034 — the byte-strings flag; `strip` whitespace

- Gap: Java reads the semantics flag `-internal_bazel_only_utf_8_byte_strings`
  (StarlarkSemantics.java:267; Bazel sets it from `--internal_starlark_utf_8_byte_strings`,
  default true, BuildLanguageOptions.java:801) in two places: `StringModule.matcher` (:326; `strip`,
  `lstrip`, `rstrip` without `chars` remove ASCII whitespace, :206, when on and Latin-1
  whitespace, :196, when off) and `Json.decode`. starlark-rust has no semantics and stripped Rust
  whitespace (U+0085 and U+00A0, UTF-8 continuation bytes, were stripped; U+001C..U+001F were
  not).
- Fix: `eval/runtime/evaluator.rs`: `Evaluator::set_utf8_byte_strings(bool)` /
  `utf8_byte_strings()` (default true, Bazel's default). `values/types/string/methods.rs`:
  `strip`/`lstrip`/`rstrip` take the evaluator (no longer `speculative_exec_safe`: the result
  depends on the flag) and use `java_strip_matcher` / `java_strip_whitespace`.
- Users: blaze_starlark `eval` sets the flag from `StarlarkSemantics::utf8_byte_strings()` on
  every evaluator it creates; blaze_starlark `json.decode` reads it; starlark_conformance sets it
  per ScriptTest mode. Evaluators created elsewhere get Bazel's default.
- Tests: `patch_0034_strip_whitespace_flag`; `java_recorded_unicode_latin1_chars` (flag off,
  against Bazel's interpreter); ScriptTest_Latin1 string_misc.star:220-222.

## 0035 — string repr

- Gap: Java `Printer.escapeCharacter` (net/starlark/java/eval/Printer.java:239) escapes `"`,
  `\`, `\r`, `\n`, `\t` and other chars below 0x20 (as `\xNN`) and writes every other char
  as it is. starlark-rust also escaped DEL, U+0080..U+00FF (the non-ASCII bytes of every internal
  string: `repr("é")` came out as `"\xc3\xa9"`) and non-alphanumeric chars above (`\uNNNN`,
  `\UNNNNNNNN`). Affects `repr`, `str` of containers, `%r`, error messages that quote a string.
- Fix: `values/types/string/repr.rs` `need_escape` / `push_escape` (the vendored unit tests
  updated to Java's output).
- Users: blaze_starlark `printer::quote` already did the same for Blaze values (Label repr now
  calls it).
- Tests: `patch_0035_repr_escapes`; dialect `bazel_string_repr_escapes_only_control_chars`;
  `byte_string_cases_match_oracle` (`repr_e_ctl`, `str_list`, `pct_s_r`).

## 0036 — string repeat limit counts chars

- Gap: 0002 checked `"s" * n` against `i32::MAX` UTF-8 bytes; Java checks `s.length()` chars
  (EvalUtils.java:433 repeatString), which for an internal string is its byte count. Each
  non-ASCII char of an internal string is two UTF-8 bytes, so the limit and the message's
  length differed.
- Fix: `values/types/string/str_type.rs` `mul` counts chars (`fast_string::len`).
- Test: `patch_0036_repeat_limit_counts_chars`.

## 0037 — lexer whitespace

- Gap: starlark-rust lexed a tab between tokens as an error token ("tabs are not allowed": `x =\t1`,
  `x = 1\t# c`, a tab after a line continuation or inside brackets); Java ignores tabs, spaces
  and carriage returns between tokens (net/starlark/java/syntax/Lexer.java:824-828; inside
  brackets newlineInsideExpression, :191). Java rejects a tab anywhere in the indentation it
  scans (Lexer.computeIndentation, :217-220), which includes blank lines, lines holding only a
  comment and whitespace at the end of the file; starlark-rust only rejected tabs before a
  token. A lone carriage return (between tokens, in a comment, `\r\r\n`, inside brackets) was
  "invalid input"; Java skips it, and a comment runs to the line feed (scanToNewline, :555).
- Fix: `starlark_syntax/src/lexer.rs`: `Token::Tabs` is skipped in `next()`; `calculate_indent`
  remembers the first tab of the whole scan and fails at the position after it (Java's
  position) on every exit; `#[logos(skip r"\r")]`; the comment regex is `#[^\n]*` (a trailing CR
  of a CRLF line is still dropped from the content); `LexemeError::InvalidTab` has Java's text
  "Tab characters are not allowed for indentation. Use spaces instead.".
- Tests: `patch_0037_lexer_whitespace`; `java_recorded_interp` (tests/data/interp/lex_tab_*,
  lex_cr_*, recorded on Bazel's interpreter); `scripts/starlark/interp-diff binary` (the same
  cases as BUILD and .bzl files on the oracle and Blaze). Vendored test updated:
  `tests/uncategorized.rs` `test_tabs_fail`.

## 0038 — number literals

- Gap: `0b101` / `0B1` were binary literals; Bazel's lexer scans them (Lexer.java:945-953) and
  IntLiteral.scan rejects every `0` prefix but `0x`/`0o` ("invalid octal literal: 0b101 (use
  '0ob101')", IntLiteral.java:69-84). A float literal beyond the double range (`1e999`) was
  `+inf`; Java: "floating-point literal too large" (Lexer.java:1005-1008).
- Fix: `starlark_syntax/src/lexer.rs`: `RawBinInt` and a decimal with a leading 0 fail with
  `LexemeError::BlazeInvalidOctalLiteral` (Java's text, also for `01`, which was "integer cannot
  have leading 0"); a non-finite `Float` fails with `BlazeFloatTooLarge`.
- Tests: `patch_0038_number_literals`; interp cases lex_binary_literal*, lex_int_00,
  lex_float_too_large. Vendored golden `lexer_tests/int_lit` updated (binary literals removed,
  the leading-0 text).

## 0039 — `async` and `await`

- Gap: starlark-rust reserved `async` and `await`; Java's keywordMap (Lexer.java:569-599) has
  neither, so `async = 1` is valid in Bazel.
- Fix: `starlark_syntax/src/lexer.rs` `Token::Reserved` regex without them.
- Test: `patch_0039_async_await_identifiers`; interp case lex_async_ident.

## 0040 — no bytes literals in Bazel's dialect

- Gap: `b"abc"`, `br'..'`, `rb".."` were bytes literals; Bazel has no bytes type and its lexer
  reads `b` as an identifier followed by a string literal (Lexer.java:855-877), a syntax error.
- Fix: `starlark_syntax/src/dialect.rs`: `Dialect::enable_bytes_literals` (true in `Standard`,
  `Extended`, `AllOptionsInternal`: upstream behaviour). `lexer.rs`: when false, the prefix is an
  `Identifier` and the quote starts an ordinary (non-raw) string literal
  (`bytes_prefix_as_identifier`), exactly Java's token stream.
- Users: blaze_starlark `FileKind::dialect_with_options` sets it false for every file kind.
- Test: `patch_0040_no_bytes_literals`; interp cases lex_bytes_*.

## 0041 — string index clamping

- Bug: `convert_str_indices` returned `None` ("not found" / `False` / 0) when `start` was past
  the end or after `end`; Java StringModule.substringIndices (StringModule.java:111-126) clamps
  each index (EvalUtils.toIndex) and moves an end before the start to the start, so an empty
  needle is found there: `"abc".find("", 5)` 3 (was -1), `"abc".count("", 2, 1)` 1 (was 0),
  `"abc".startswith("", 5)` True (was False).
- Fix: `starlark_syntax/src/fast_string.rs` `convert_str_indices` is Java's algorithm and always
  returns `Some` (signature unchanged); `convert_str_indices_slow` is unused. Affects `find`,
  `rfind`, `index`, `rindex`, `count`, `startswith`, `endswith` and `s[a:b]` (same clamping as
  Java Starlark.slice for step 1).
- Test: `patch_0041_string_index_clamping`; interp case str_values (with every method's clamping).

## 0042 — `None` bounds in string slices

- Bug: `"abc"[None:2]` failed (the string fast path unpacked the bound as an int); Java
  Starlark.slice treats `None` as the default (Starlark.java:692, :698). Lists, tuples and the
  string stride path already did.
- Fix: `values/types/string/str_type.rs` `slice`.
- Test: `patch_0042_string_slice_none_bounds`; interp case str_values.

## 0043 — no `codepoints`

- Gap: strings had a `codepoints` method; Java's StringModule has none (`dir("")` differed and
  `"abc".codepoints()` was accepted).
- Fix: `values/types/string/methods.rs`: method removed; `iter.rs` `iterate_codepoints` unused.
- Test: `patch_0043_no_codepoints`; interp cases str_codepoints, str_values (`dir("")`).

## 0044 — `int()`

- Gap: `int()` returned 0 (Java: `x` is required, MethodLibrary.java:551-570); a string was
  parsed with base 0 (prefix inference) by default (Java: 10, so `int("0x10")` fails);
  `int("010", 0)` and `int("00", 0)` were 10 and 0 (Java: "cannot infer base when string begins
  with a 0"); `int("1_000")` was 1000 (num-bigint accepts `_`; Java's Long.parseLong /
  BigInteger do not). Texts were starlark-rust's.
- Fix: `values/types/int/globals.rs`: `x` required; `java_parse_int` ports StarlarkInt.parse
  (StarlarkInt.java:99-168) with its texts ("empty string", "invalid base N (want 2 <= base <=
  36)", "invalid base-N literal: \"s\"", "cannot infer base ..."), as value errors; "can't
  convert non-string with explicit base" (MethodLibrary.java:624). Digits are
  `Character.digit`, which for chars up to U+00FF (every char of an internal string) is ASCII
  only; chars above (Unicode digits Java accepts, unreachable from Blaze) are rejected.
  `StarlarkInt::from_str_radix` is unused.
- Test: `patch_0044_int_parse`; interp cases str_int_*, more_i_*, more_values.

## 0045 — `str.format` fields

- Gap: `"{!r}".format(1)` and `"{0!r}".format("a")` used Python's conversions; Java's
  FormatParser has none: the whole field (`!r`, `0!r`, `x!s`, `:d`) is the key, so these fail
  "Missing argument" (FormatParser.java:193 getFieldName). A field of digits and `-` is an index
  when Integer.parseInt accepts it (`{-0}` is index 0, `{-1}` fails "No replacement found",
  `{99999999999}` is a keyword), FormatParser.java:85-100.
- Fix: `starlark_syntax/src/dot_format_parser.rs`: `FormatParser::new` has no conversions;
  `new_with_conversions` (upstream) is used only for f-strings. `values/types/string/dot_format.rs`
  `format_capture` / `FormatArgs::by_index` take Java's numeric rule (i32 index). f-strings
  desugar to `.format`, so `eval/compiler/expr.rs` turns an f-string's `{!r}` field into `{}`
  with the argument `"%r" % (x,)` (`fstring_repr_fields`); f-strings are disabled in Bazel's
  dialect, this keeps upstream's.
- Tests: `patch_0045_format_fields`; interp cases str_format_*, more_f_*; vendored
  `dot_format_parser` tests (`test_parser_bazel_fields_have_no_conversions`) and
  `test_parse_format_one` updated, `format` doc example changed.

## 0046 — `strip(None)`

- Bug: `" a ".strip(None)` (and `lstrip`, `rstrip`) failed; Java's `chars` is String or
  NoneType, default None (StringModule.java:243-320).
- Fix: `values/types/string/methods.rs`: `chars: NoneOr<&str>`.
- Test: `patch_0046_strip_none_chars`; interp case str_values.

## 0047 — `...` only in type expressions

- Gap: with Bazel's type syntax (0020) `x = ...` and `def f(x = ...)` were accepted in .bzl
  files; Java's Parser rejects an ellipsis outside a type expression ("ellipsis ('...') is not
  allowed outside type expressions", Parser.java:775-778; insideTypeExpr is set by
  parseTypeExprWithFallback, :1084-1095, and cleared, not restored, after it). Found by
  `scripts/starlark/interp-diff binary` (oracle rejects the .bzl).
- Fix: `starlark_syntax/src/syntax/parser_rd.rs`: `in_type_expr` set by `parse_type_expr`; an
  `Ellipsis` atom under `type_syntax` outside it is an error (recorded, parsing continues, as in
  Java). `DialectTypes::Disable` (validate.rs) and `Enable` are unchanged.
- Test: `patch_0047_ellipsis_only_in_type_expressions`; interp-diff cases lex_ellipsis,
  lex_ellipsis_in_type, lex_ellipsis_default (.bzl packages).

## 0048 — `cast` and `isinstance` in call arguments

- Bug: under type syntax a call argument starting with `cast` / `isinstance` was parsed as a
  call of a name (`parse_argument` consumed the identifier before 0020's keyword check), so
  `print(cast(int, 3))` failed "Variable `cast` not found" (Bazel prints 3) and `f(cast = 1)` was
  a named argument; `load(":x.bzl", cast = "y")` bound `cast`. In Java both words are keywords
  (Lexer.java:615) and parseArgument / parseLoadStatement expect an IDENTIFIER.
- Fix: `starlark_syntax/src/syntax/parser_rd.rs` `parse_argument` and `parse_load_symbol` take the
  identifier path only when `at_type_keyword()` is `None`.
- Test: `patch_0048_type_keywords_in_arguments`; interp-diff cases lex_cast_argument,
  lex_cast_named_argument (.bzl packages).

## 0049 — Bazel's spelling suggestions

Task: Starlark core-language error texts (docs/decisions.md 2026-11-19: ported where Bazel's tests
and users read them).

- Gap: starlark-rust's `did_you_mean` used strsim Levenshtein with a 1/2 distance limit, case
  sensitive, and the first minimum in its own order. Java `SpellChecker.suggest`
  (net/starlark/java/spelling/SpellChecker.java:89) lowers both sides (`String.toLowerCase`),
  limits the distance to `min(5, (len+1)/2)`, keeps the first strictly better candidate in the
  order it is given, and Resolver.use passes a `HashSet` (Resolver.java:1266 getAllSymbols), so
  ties go to the first in Java's hash-set order.
- Fix: new module `starlark::errors::spell_checker`: `edit_distance`, `suggest`, `did_you_mean`
  (Java's), `java_string_hash_code`, `java_hash_set_order` (bucket = spread hash & (capacity-1),
  capacity 16 doubled above 3/4 full, insertion order within a bucket; a bucket of 8+ that Java
  turns into a tree is not modelled).
- Users: 0050, 0051 (attributes), 0052 (keyword arguments).
- Test: `patch_0049_spell_checker` (Java SpellCheckerTest's cases); the module's unit tests.

## 0050 — name resolution texts

- Gap: "Variable `x` not found[, did you mean `y`?]" (Java: "name 'x' is not defined" and
  " (did you mean 'y'?)", Module.java:256, Resolver.java:950); "Local variable `x` referenced
  before assignment" for locals and globals (Java: "%s variable '%s' is referenced before
  assignment.", Eval.java:825, scope `local`/`global`).
- Fix: `eval/compiler/scope.rs` `ScopeError::NameNotDefined`, candidates in Java's order
  (locals, globals/predeclared/universe, file globals) through `java_hash_set_order`;
  `eval/runtime/evaluator.rs` `EvaluatorError::VariableReferencedBeforeAssignment`. Java says
  `cell`/`free` for a captured variable; starlark-rust's slots do not record it (`local`).
- Users: blaze_packages `builtins::env::undefined_variable_name` parses this text (it still
  expects the old one: see the task report).
- Test: `patch_0050_name_errors`; interp cases err_res_undefined*, err_res_*_before_assign.

## 0051 — value and builtin error texts

- Gap: starlark-rust's texts for every failing operation ("Operation `+` not supported for types
  `int` and `string`", "Index `5` is out of bound", "Key `"b"` was not found", "Value of type `list`
  is not hashable", "Object of type `int` has no attribute `foo`", "Unpacked 2 values but
  expected 3", "This operation mutates an iterable for an iterator while iterating.", ...).
- Fix (Java site at each change):
  - `values/error.rs`: `ValueError::Bazel(String)`; `unsupported_owned` maps its operation names
    to Java's texts (EvalUtils.binaryOp/unaryOp/index, Starlark.compareUnchecked/slice/
    toIterable/getStarlarkCallable, setField, `len`'s parameter check);
    `ValueError::bazel_no_attr` ("'T' value has no field or method 'x'" + suggestion over the
    sorted `dir`), used by `get_attr_error`, the compiled `.` and `getattr`;
    `ControlError::NotHashableValue` "unhashable type: 'T'".
  - `values/index.rs`: `java_to_int` (Starlark.toInt: "got T for WHAT, want int", "got N for
    WHAT, want value in signed 32-bit range"), `java_sequence_index` (EvalUtils.getSequenceIndex);
    sequence/string/list indices, slice bounds and step use Java's names.
  - `eval/bc/instr_impl.rs` sequence assignment (Eval.assignSequence; strings are not
    iterable here), `eval/compiler/expr.rs` duplicate dict key.
  - Mutation during iteration: "T value is temporarily immutable due to active for-loop
    iteration" (list, dict, set).
  - `dict[k]` of an unhashable key is "key [] not found in dictionary" (Dict.getIndex).
  - `values/types/string/interpolation.rs` `percent`: Java's Printer.formatWithList (its order
    of checks and texts); `dot_format.rs` `format`: Java's FormatParser.
  - Builtins: `float()` (MethodLibrary.floatForStarlark with Java's `Double.parseDouble`
    grammar: whitespace, `d`/`f` suffixes, hex floats), `range()` (Java's size check, `step
    cannot be 0`; `Range::length` in 64-bit arithmetic as RangeList), `min`/`max`, list
    `index`/`remove`/`pop`, dict `pop`/`popitem`/`update`/`dict()` (Dict.update's texts), str
    `index`/`rindex`/`partition`/`rpartition`, shifts (`toInt("shift count")`, "negative shift
    count: N", checks before the zero shortcut), repeat counts (`toInt("repeat")`).
  - Semantics the texts depend on: `list += x` extends only for a list (Eval.inplaceBinaryOp;
    otherwise the binary `+` and its error); list/tuple comparison tests element equality first
    (Sequence.compare), so `(None,) < (None,)` is False.
- Not done: Java's TimSort comparison order in `sorted` (which pair fails first), `RangeList`
  slices in 64-bit arithmetic (range.star:41-49), "cell"/"free" scopes.
- Test: `patch_0051_value_errors`; interp cases err_op_*, err_ix_*, err_hash_*, err_mut_*,
  err_call_* (java_recorded_interp).

## 0052 — call argument errors

- Gap: one argument binder with its own texts ("Missing parameter `b` for call to `f`", "Found 1
  extra positional argument(s) for call to ...", "Argument `a` occurs more than once", "Wrong
  number of positional arguments, expected 1, got 2", "Type of parameter `x` doesn't match,
  expected `str`, actual `int (repr: 1)`"); parameter names of the core builtins differed from
  Java's (`len(a)`, `int(a)`, `list.append(el)`, `range(a1, a2)`, `str.find(needle)`, ...).
- Fix:
  - `eval/runtime/params/spec.rs`: `ParametersSpec::java_argument_error` replays Java's argument
    processing (Eval.evalCall order: positional, named, `*args`, `**kwargs`; then the callee's
    `call`) for a failed call and returns Java's first error, in StarlarkFunction's wording for
    a `def`/`lambda` (`java_def_name`, set by `InstrDefImpl`) and BuiltinFunction's otherwise;
    `*args`/`**kwargs` value errors (Eval.java:735-754).
  - `__derive_refs/parse_args.rs` + starlark_derive `render/fun.rs`: the positional-only fast
    path of `#[starlark_module]` functions reports through the same diagnosis.
  - `values/unpack.rs` `BazelParamTypeError`: "in call to F(), parameter 'P' got value of type
    'T', want 'W'" (the function filled in by `NativeFunc::invoke` / `NativeMeth::invoke`);
    `java_allowed_types` gives `W` from the parameter's type (Java's order for the core
    builtins whose order differs; otherwise `NoneType` last); an int beyond int32 is Java's
    `toInt` text; `java_check_iterable` for the parameters Java declares `StarlarkIterable`
    (`all`, `any`, `sorted`, `reversed`, `list`, `tuple`, `set`, `list.extend`, `str.join`);
    `str.join` element text; `startswith`/`endswith` element text (Sequence.cast); `key=None`
    accepted by `sorted`/`min`/`max` (Java: callable or NoneType); `range`'s parameters.
  - Java's parameter names for the core builtins (each renamed parameter is bound to its old
    variable name on the first line of the body).
- Not done: Java checks a builtin's argument types while binding (before arity errors);
  starlark-rust after; `print(sep=)`; the `callable` type check of `key`; a positional argument
  from `*args` that rebinds a named one is accepted by Java (StarlarkFunction.addPositionalArg)
  and rejected here.
- Test: `patch_0052_argument_errors`; interp cases err_def_*, err_call_*.

## 0053 — lexer texts

- Gap: "Parse error: invalid input `$`", "Parse error: incorrect indentation" (at the previous
  line's end), "Parse error: cannot use reserved keyword `import`", `0x`/`1e`/`0o9` lexed as two
  tokens.
- Fix: `starlark_syntax/src/lexer.rs`: Java's texts (Lexer.java:876 "invalid character: 'c'",
  :256 "indentation error" at the last char of the indentation, Parser.java:425 forbidden
  keywords); `Token::BlazeBadNumber` for the number literals Java's lexer rejects ("invalid hex
  literal", "invalid binary literal", "invalid base-8 integer literal: 0o9", "invalid float
  literal"), at the literal's start.
- Test: `patch_0053_lexer_errors`; interp cases err_lex_*, lex_*.

## 0054 — parser texts

- Gap: "Parse error: unexpected X, expected Y".
- Fix: `starlark_syntax/src/syntax/parser_rd.rs`: `error_expected` prints Java's
  `syntax error at 'TOKEN': expected KIND` (Parser.java:334; `tokenString`: a string literal's
  value in double quotes, identifiers and numbers by value, `Double.toString` for floats, else
  TokenKind's text; "indentation error" at an unexpected indent); `expect_java`/`error_java` for
  Java's context messages: list (`expected ',', 'for' or ']'`), comprehensions (`expected ']',
  'for' or 'if'`), call arguments (`expected ,`), load (`expected at least one symbol to load`,
  `expected string`, `expected either a literal string or an identifier`), conditional (`missing
  else clause ...` at the expression), generator expressions, implicit string concatenation,
  `expected an indented block`, non-associative comparisons.
- Limitation: Java's parser recovers and reports every syntax error of a file (plus the
  resolver's errors on the recovered tree, e.g. "contains syntax errors"); starlark-rust's stops
  at the first. Porting it means porting Java's Parser recovery (syncTo/syncPast, error
  expressions) into parser_rd.rs and resolving the partial tree. The first error equals Java's
  in every recorded case but four (a lone backslash twice, an f-string, `x ==` at end of file:
  tests/data/interp/KNOWN_DIFFERENCES).
- Test: `patch_0054_parser_errors`; interp cases err_parse_*.

## 0055 — static-check texts

- Gap: starlark-rust's texts for checks Java's Resolver/Parser make ("positional argument after
  non positional", "repeated named argument", "Parameter after kwargs", "`for` cannot be used
  outside `def` in this dialect", "left-hand-side of assignment must take the form ...",
  "unparenthesized tuple with trailing comma", "type annotations are not allowed in this
  dialect", ...).
- Fix: `syntax/call.rs` (Resolver.java:708 visit(CallExpression), Java's order), `syntax/def.rs`
  (Resolver.java:1049 parameter order and duplicates, bare `*`), `syntax/validate.rs` (top-level
  `for`/`if`, `break`/`continue`, `return`, `load` placement, `...`), `syntax/grammar_util.rs`
  ("cannot assign to 'EXPR'" with the source text standing in for Java's NodePrinter, augmented
  assignment at the operator, trailing comma at the next token, "syntax error at ':': type
  annotations are disallowed" at the `:`/`->`).
- Test: `patch_0055_static_errors`; interp cases err_res_*, err_parse_*.

## 0056 — sorted, min, max, range slices, duplicate keywords

- Gap: `sorted` used Rust's `sort_by`, which compares other pairs than Java's TimSort (the first
  pair that cannot be compared names the error: Bazel says "string <=> int" where Blaze said
  "int <=> string") and panics on the inconsistent order a failed comparison leaves
  (`sorted` of 80 mixed values crashed the probe); `min(..., key=)` compared the kept key with
  the new one (Java's `Stream.max` with `ORDERING.reverse()` compares the new one first);
  `key=` of sorted/min/max accepted any value and failed late ("'int' object is not
  callable"); a range slice whose step times the range's step leaves 32 bits failed with
  "Integer overflow" (Java: one element); a name given twice to a builtin's `**kwargs` said
  "Argument `a` occurs more than once".
- Fix: `stdlib/funcs/java_timsort.rs` (new: java.util.TimSort, stopping at the comparator's first
  error) used by `sorted` with Java's reversed comparator for `reverse=True`
  (MethodLibrary.java:277); `min_max.rs` (MethodLibrary.java:119 findExtreme);
  `values/unpack.rs` `java_check_callable_or_none` ("in call to sorted(), parameter 'key' got
  value of type 'int', want 'callable or NoneType'", BuiltinFunction.java:404 checkParamValue);
  `range_type.rs` slice (RangeList.java:182 getSlice: 64-bit step, clamped, 32-bit `at`);
  `arguments.rs` `FunctionError::RepeatedKwarg`, named by the builtin in
  `BazelParamTypeError::with_function` (BuiltinFunction.java:343 addNamedArg).
- Test: interp cases err_call_sorted_mixed*, err_call_sorted_key_mixed_long,
  err_call_sorted_reverse_mixed, err_call_min_*, err_call_max_key_mixed,
  err_call_sorted_key_noncallable, err_call_dict_dup_*, range_slice_overflow,
  sorted_large_stable (recorded on Bazel). The unit test `java_timsort::tests::sorts_stably`
  does not run in the workspace (vendored crate).

## 0057 — `load()` of a missing symbol

- Gap: `load(":a.bzl", "fooo")` failed with "Module has no symbol `fooo`, did you mean `foo`?",
  and loading a name the loaded file only binds by its own `load` with "Module symbol `foo` is
  not exported" (review 2026-11-25-loading-2 F1).
- Fix: `environment/modules.rs` `load_symbol` (Java Eval.java:261 execLoad): the loaded file's
  global (any visibility, not a `load` binding, Java `Module.getGlobal`), else
  `file 'M' does not contain symbol 'S'` + `SpellChecker.didYouMean` over the file's globals in
  binding order (`FrozenModule::java_globals`, Java `Module.getGlobals().keySet()`; `M` as
  written in the load statement). The names a module binds by `load` are recorded
  (`Module::add_load_bound_name`, `FrozenModuleData::load_bound`): Bazel's load bindings are
  file-local (Resolver.java:1170; only the BUILD prelude binds them globally, and no `load`
  reads the prelude's key). `eval/compiler/module.rs` locates the error one char after the start
  of the original name's string literal (Parser.java:1442/1460). `CannotImportPrivateSymbol`
  carries Java's text (Resolver.java:610; Java's resolver reports it first).
- Test: `patch_0057_load_missing_symbol`; oracle probes in the loading-fix-1 report.

## 0058 — parameter type errors: Java's allowed types

- Gap: the "want '...'" part of "in call to F(), parameter 'P' got value of type 'T', want '...'"
  was derived from starlark-rust's parameter type (0052), which does not keep Java's
  `@Param(allowedTypes)` order or names (`sequence` for a `list | tuple` parameter, Bazel's class
  names) (review 2026-11-25-loading-2 F6).
- Fix: `values/java_param_types.tsv` lists `ParamDescriptor.getTypeErrorMessage` for every
  `@StarlarkMethod` parameter of Bazel 9.3.2 (receiver `@StarlarkBuiltin` name, method,
  parameter, text; `-` for an `Object` parameter), computed by Bazel's own classes from the
  oracle's `A-server.jar` (`scripts/java-param-types/run`). `values/unpack.rs`
  `java_param_type_error_message` takes the receiver's entry (methods: `NativeMeth::invoke`
  passes the receiver's `type()`), else the text all of Bazel's methods of that name agree on,
  else 0052's derivation (a builtin Bazel does not have, or an `Object` parameter).
- Limitation: Java checks named arguments in the order of the call (BuiltinFunction.java:376
  addNamedArg); the derived unpacking checks them in the order of declaration, so a call with
  two ill-typed arguments can name the other one.
- Test: oracle probes in the loading-fix-1 report.

## 0059 — no `[]` on builtin functions

- Gap: `list[int]` and `set[int]` evaluated to starlark-rust type values; Bazel 9.3.2 fails with
  "type 'builtin_function_or_method' has no operator [](builtin_function_or_method)" (oracle
  probe q89 of the loading-fix-1 report). Bazel does not evaluate type expressions (patch 0020).
- Fix: `values/types/function.rs` `NativeFunction::at` is Java's unsupported-index error for
  every builtin (EvalUtils.index); `TypeCompiled::type_list_of`/`type_set_of` kept, unused.
- Test: `patch_0059_builtin_has_no_index`.

## 0060 — comparison nesting guard

- Gap: comparing nested values deeper than 200 levels (debug builds) or 3000 (release) failed
  with "Too many recursion levels"; Java has no limit (Bazel 9.3.2 compares two 5000-deep lists
  with `==`, oracle probe q87; at 5000 `<` overflows the JVM stack and crashes Bazel, q88).
- Fix: `values/stack_guard.rs` `MAX_RECURSION = 20000` in every build: deep enough for what Bazel
  accepts, still a clean error instead of a native stack overflow on Blaze's 256 MiB evaluation
  stack (`blaze_starlark::eval::EVAL_STACK_SIZE`). PORT: the depth where Java gives up depends on
  the JVM's stack; Blaze answers some comparisons Bazel crashes on.
- Test: `patch_0060_deep_comparison` (15000 levels, `==` and `<`, on a 256 MiB stack).

### 0058, round 2 (review loading-2 round 2, R2)

- The table is printed by calling Bazel's code: `ParamTypes.java` reaches the package-private
  `MethodDescriptor.of` and `ParamDescriptor.getTypeErrorMessage` / `getAllowedClasses` by
  reflection, for every class of the server jar with Starlark methods found as
  `CallUtils.buildStarlarkClassDescriptor` finds them (`Class.getMethods()`, so a method a
  builtin class inherits from an unannotated base, as `repository_ctx` and `module_ctx` from
  `StarlarkBaseExternalContext`, is listed under that class). 1,328 rows (the 1,213 of round 1,
  unchanged, plus 115 inherited ones).
- A function call (no receiver) takes the row of receiver `""` (Java's global functions) before
  the "all rows agree" rule: `configure_features(requested_features=)` is `list`.
- Test: `blaze_sky_top` `tests::java_param_types_test` walks every builtin Blaze registers in its
  BUILD, .bzl, MODULE.bazel, builtins and .scl environments (with namespaces and methods
  reachable from them) and requires each typed parameter to have its row or a reasoned
  exception.

### 0060, round 2

- The guard also stops while less than 1 MiB of the current thread's stack is left (glibc
  `pthread_getattr_np`, declared directly; Blaze runs on Linux x86-64 only): Blaze's
  execution-phase evaluator threads have 16 MiB and its gRPC command threads 8 MiB or more, and
  comparison and hashing run there too. Hashing a value is guarded as comparison is (a nested
  tuple's hash recurses).
- Test: `patch_0060_guard_follows_the_thread_stack`.

## 0061 — call-site error locations

- Gap (review loading-2, findings N1 and the three traceback items): Java's `Eval.evalCall`
  looks the callee up before evaluating any argument and fails "'T' object is not callable" in
  the caller; it binds the arguments of a call with named or `*`/`**` arguments with
  `ArgumentProcessor.addPositionalArg` / `addNamedArg`, whose errors leave `evalCall` without a
  location, so the innermost enclosing node that sets one gives it (an assignment's operator, a
  `for` statement's or comprehension clause's `for`, a dict comprehension value's `:`; the `.`
  of `x.f op= y`), else the frame keeps the `(`; `*x` and `**x` fail in the caller at the
  `*`/`**`. starlark-rust located all of these at the `(` with the callee's frame
  (`Error in 1:`, `Error in len:` for `len(*1)`).
- Fix: `eval/runtime/java_call.rs` (new): `JavaCallSpans` (handler, `*`, `**`, positional-only)
  attached to each call expression's `FrameSpan` by the compiler (`Compiler::java_handler`, set
  around assignment right-hand sides, `for` bodies, comprehension clauses and dict values, reset
  in function bodies); `Evaluator::with_call_stack` relocates an error the first time the call
  stack is captured: a binding error (`FunctionError::JavaBinding` from 0052's replay,
  `BazelParamTypeError`, `RepeatedKwarg`, or a host builtin's
  `starlark::values::bazel_argument_binding_error`) gets the handler as the callee's call site,
  a `*`/`**` error (`FunctionError::JavaStar`/`JavaStarStar`) or a non-callable callee
  (`JavaNotCallable`, the default `StarlarkValue::invoke`) loses the callee's frame and is
  located at the `*`/`**`/handler. `starlark_syntax` `Error::call_stack_mut`, `replace_span`.
- Limitations: without a handler Java keeps the frame's last location, which is the `(` of the
  last call evaluated before the error; Blaze uses this call's `(`. Equal for a binding error
  whose arguments contain no call (the call sets its own `(` first); different for a
  non-callable callee in an expression statement (Java shows the previous call's `(`, oracle
  probe t08). Java checks callability
  before evaluating the arguments; Blaze evaluates them first (only an argument that itself
  fails tells the difference).
- Test: oracle probes t01-t26 of the loading-fix-2 report; `traceback_locations_test`.

## 0062 — argument checks in Java's order

- Gap (review loading-2, finding N2): `len(1, 2)` reported the arity; Java checks the bound
  argument's type first (`fastcall`/`addPositionalArg` before `call`), and checks named arguments
  in the order of the call, where starlark-rust unpacks in the order of declaration.
- Fix: `values/types/function.rs` `java_argument_order`: when a native function or method failed
  with an arity/missing-argument error (`FunctionError::Bazel` of 0052's `call`) or a parameter
  type error, the first argument, in Java's order (positional, then named as written), whose
  value is none of the parameter's allowed classes (`values/unpack.rs` `java_param_accepts`, from
  0058's table; the documented derived type where the table has no row) gives Java's type error
  (`NativeFunction::invoke`, frozen native calls, bound methods, known methods).
- Limitation: an unexpected or repeated keyword raised by binding is not reordered against the
  type checks of earlier arguments.
- Test: oracle probes t12-t15 of the loading-fix-2 report.
- Round 2 (review query-2, item 2): the table's class names are matched against Starlark type
  names, which differ for a Java interface: `CqueryNode` (cquery's `build_options(target)`,
  `providers(target)`) is implemented by every configured target class, whose type is `Target`.
  `java_param_accepts` maps `CqueryNode` to `Target` (`BLAZE PATCH 0062 (round 2)`).
- Test (round 2): configured_query_test#test_starlark_build_options_invalid_arg
  (`build_options(target, 'blah')`: Java's arity text).
- Round 3 (query-4): the table's other Java class names that are not Starlark type names.
  `StarlarkValue` is every value but a string or a bool (Java's `String` and `Boolean` are not
  `StarlarkValue`s); `iterable` (Java's `StarlarkIterable`) and `Iterable` (`java.lang.Iterable`)
  are a list, tuple, dict, set or range (round 1 also accepted a depset, which Java's
  `Depset` is not). The provider interfaces are the embedder's: `register_java_interface_matcher`
  takes a `JavaInterfaceMatcher` (`fn(&str, Value) -> Option<bool>`, `None` for a name it does
  not know), and Blaze registers one in `blaze_analysis::starlark::install_hooks` for `Info`,
  `NativeInfo`, `Provider` and `structure` (`blaze_analysis::info::java_interface_matcher`).
  Rows: `cc_common.*(cc_toolchain: Info)`, `java_common.*(java_toolchain, plugin_info: Info)`,
  `java_common.check_provider_instances(provider_type: Provider)`,
  `proto.encode_text(x: structure or NativeInfo)`, `cc_internal.freeze(value: StarlarkValue)`,
  `repository_ctx.download(url: string or Iterable)` and the `module` / override `Iterable`s.

## 0063 — key-tracking immutable dicts

- Gap: Java `Dict.Builder.buildImmutableWithKeyTracking` (net/starlark/java/eval/Dict.java:532)
  makes an `ImmutableKeyTrackingDict` (Dict.java:789): an immutable dict whose overridden
  `containsKey`, `get`, `keySet` and `entrySet` record the keys accessed (present keys only).
  `StarlarkRuleContext.var()` builds `ctx.var` this way when required config fragments are
  tracked, and `lookedUpVariables()` reports the `--define` variables a rule read
  (`cquery --show_config_fragments`).
- Fix: `types/dict/value.rs`: `MutableDictData` gets `accessed_keys: RefCell<Option<Vec<usize>>>`
  (indexes of the keys, in order of first access; `Some` for a key-tracking dict), and the
  `DictLike` trait two hooks (`record_key_index_access`, `record_all_keys_access`, no-ops for
  other dicts). Recorded as Java does: `in` (`is_in`), `d[k]` (`at`) and `d.get(k)` record the key
  when present; iteration (`iterate`, Java `iterator()` = `keySet()`), `repr`/`str` (Java
  `printList(entrySet())`), `d | x` and `x | d`, `x |= d`, `x.update(d)`, `dict(d)` (Java copies
  through `entrySet()`) and `f(**d)` (Java iterates `entrySet()`) record every key. `keys()`,
  `items()`, `values()`, `len` and `==` record nothing (Java reads `contents` directly).
  `alloc_immutable_key_tracking_dict(heap, Dict) -> Value`, `dict_accessed_keys(value) ->
  Option<Vec<Value>>` (Java `getAccessedKeys`), `dict_record_all_keys_accessed(value)`; re-exported
  from `starlark::values::dict`. `dict.get`'s receiver is a `Value` (was `DictRef`). Freezing
  drops the tracking (Java reads the keys before the context is closed).
- Limitation: native functions that read a dict through `DictRef` (Java natives that call
  `Dict.cast`, which iterates `entrySet()`) record nothing; an empty key-tracking dict is not the
  shared empty frozen dict (Java's builder makes a new dict too).
- Users: blaze_analysis `starlark_rule_context::get_var` / `looked_up_variables`.
- Test: `patch_0063_key_tracking_dict`.

## 0064 — the running evaluator's `extra`

- Gap: Java's `Starlark.str(x, semantics)` passes the converting thread's semantics to the value
  (`Label.str`: `--incompatible_unambiguous_label_stringification`); starlark-rust's
  `collect_str` has no evaluator.
- Fix: `eval/runtime/evaluator.rs`: a thread-local holds the `extra` of the evaluator running
  `eval_module` or `eval_function` (a guard sets it on entry and restores the previous one on
  return or unwinding, so nested evaluations see their own); `eval::with_current_extra(f)` gives
  it to `f` (`None` outside an evaluation).
- Users: blaze_starlark `label.rs` (`Label.str` reads the `ThreadContext`'s semantics).
- Test: `patch_0064_current_extra`.

