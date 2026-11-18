# Bazel's Starlark on starlark-rust 0.14.2

Written 2026-11-17 against Bazel 9.3.2 (5951e93c) and crates.io `starlark` 0.14.2
(`~/.cargo/registry/src/index.crates.io-*/starlark-0.14.2`, plus `starlark_syntax`,
`starlark_derive`, `starlark_map` 0.14.2). Code: `crates/starlark_conformance` (harness, spikes,
probes). Numbers come from the working tree on top of commit 251f89a. Java paths are relative to
`src/main/java` of the pinned source unless they start with `src/test`.

## 1. Verdict

starlark-rust can carry Bazel's Starlark. Nothing blocks the brotli/re2 milestone. Some gaps would
block exact behaviour, though, and none of them can be fixed from outside the crate: builtin
methods and operators of core types cannot be overridden through the public API. **Recommendation:
vendor starlark 0.14.2 (`starlark`, `starlark_syntax`, `starlark_derive`, `starlark_map`) under
`third_party/starlark-rust` with `[patch.crates-io]` and keep a short, numbered patch queue
(section 4.3).** Everything Bazel-specific (natives, struct, json, print, fail, depset, Label,
providers, BUILD checks, tracebacks) lives in `blaze_starlark` and the crates above it, using the
public API.

Evidence:

- **Parse sweep.** All 8,188 `.bzl`/BUILD files available locally parse with the Bazel dialect,
  and the BUILD files also pass the BUILD checks. The sweep covered the oracle output bases'
  external repos, the corpus checkouts and Bazel's `src/main/starlark`:
  `starlark_conformance parse ...` reported 0 failures.
- **Conformance.** Bazel's 38 ScriptTest files: 28/95 chunks pass under strict Java semantics, and
  2,356/2,714 assertions pass (86.8%) when aborted chunks are re-run statement by statement. Of
  the 473 failures, 283 are error text only, 36 are missing builtins, 19 are harness limits and
  135 are semantic. Most semantic failures cluster around 10 root causes (section 3.2).
- **Spikes.** Every integration point Blaze needs works with the public API: custom values,
  freezing, loads, a frozen function called from another thread with a `ctx`, per-call context
  in `Evaluator::extra`, rayon parallelism over shared frozen modules, and Bazel-exact tracebacks.
  The one exception is a single `unsafe` call to bring a frozen value into a new module's heap
  (section 2.6). The traceback and `print` formats were checked against the oracle.

## 2. Integration API (starlark 0.14.2)

### 2.1 Dialect (`starlark::syntax::Dialect`, starlark_syntax `dialect.rs`)

The fields are public (`_non_exhaustive: ()` is public too, so `..Dialect::Standard` works):
`enable_def`, `enable_lambda`, `enable_load`, `enable_keyword_only_arguments`,
`enable_positional_only_arguments`, `enable_types: DialectTypes {Disable, ParseOnly, Enable}`,
`enable_load_reexport`, `enable_top_level_stmt`, `enable_f_strings`.

The Bazel settings are in `starlark_conformance::dialect`:

| field | .bzl | BUILD | why |
|---|---|---|---|
| def / lambda / load | on | on | BUILD bans def/lambda via `build_check` so the error text is Bazel's |
| keyword_only (`*, b`) | on | on | Bazel Resolver accepts bare `*` |
| positional_only (`/`) | off | off | not in Bazel's grammar |
| types | Disable | Disable | `--experimental_starlark_type_syntax` is off by default |
| load_reexport | **off** | off | Bazel load bindings are file-local |
| top_level_stmt | off | **on** | .bzl: the parser rejects top-level `if`/`for`; BUILD: parse them and let `build_check` report Bazel's message |
| f_strings | off | off | not Bazel syntax |

`AstModule::parse(filename, String, &Dialect)` parses a file. The AST is public
(`starlark::syntax::ast::{StmtP, ExprP, ArgumentP, …}`, `AstModule::statement()`, `file_span`,
`loads()`), so Blaze can write its own static checks. `AstModule::replace_binary_operators` can
rewrite `op` into calls (a possible workaround for `<<`, section 4).

### 2.2 Globals and natives

`GlobalsBuilder::new()` / `::standard()` / `::extended_by(&[LibraryExtension])`. Then
`.with(fn)`, `.with_namespace("json", fn)`, `set(name, value)` (a later `set` overwrites) and
`build()`. `Globals` is `Send + Sync`, and so are `FrozenModule` and `OwnedFrozenValue` (checked
by test).

- Natives are defined with `#[starlark_module] fn natives(builder: &mut GlobalsBuilder)`, each one
  an inner `fn`:
  - Parameter attributes: `#[starlark(require = pos | named)]`, `#[starlark(default = …)]`,
    `#[starlark(args)] UnpackTuple<Value>`, `#[starlark(kwargs)]`. For raw access, use
    `args: &Arguments<'v, '_>` with `names_map()` and `positional()`.
  - Extra inputs: `eval: &mut Evaluator<'v,'_,'_>` or `heap: Heap<'v>`.
  - Return `anyhow::Result<T>` (error kind Native) or `starlark::Result<T>`. `T` must be
    `AllocValue`.
  - Useful unpack types: `NoneOr<T>`, `UnpackListOrTuple<T>` (a plain `UnpackTuple` rejects
    lists), `StarlarkCallable`, `ValueOf`.
- Methods on a type: `#[starlark_module] fn m(builder: &mut MethodsBuilder)`, where
  `#[starlark(attribute)]` marks a field. Register it from
  `StarlarkValue::get_methods() -> Option<&'static Methods>` with
  `static RES: MethodsStatic = MethodsStatic::new("Name", m)`.
- `LibraryExtension` provides `StructType`, `RecordType`, `EnumType`, `NamespaceType`, `Map`,
  `Filter`, `Partial`, `Debug`, `Print`, `Pprint`, `Pstr`, `Prepr`, `Breakpoint`, `Json` (a
  function `json(x)`, not Bazel's module), `Typing`, `Internal`, `CallStack` and `SetType`.
- The Bazel universe (`Starlark.UNIVERSE`: MethodLibrary + True/False/None, with `set` on by
  default in 9.3.2) is fully present in `standard + SetType + Print`. starlark-rust adds `bytes`,
  `chr` and `ord`. Blaze copies the names it wants from a full `Globals` into a fresh builder:
  `builder.frozen_heap().add_reference(full.heap())`, then `builder.set(name, frozen_value)`
  (`dialect::bazel_universe_builder`).
- `struct`, `record`, `enum`, `namespace` and `json` exist only as extensions, and their
  semantics are not Bazel's. Bazel's `struct` (repr `struct(a = 1)`, `+` concatenation, its error
  texts) and `json` (sorted keys, error texts, `indent`, `encode_indent`) are Bazel natives in
  `blaze_starlark`. Do not use the extensions.

### 2.3 Custom values

- **Simple value** (no `Value` inside, `Send + Sync`): derive `Debug, ProvidesStaticType,
  NoSerialize, Allocative`, implement `Display`, call `starlark_simple_value!(T)`, then write
  `#[starlark_value(type = "Label")] impl<'v> StarlarkValue<'v> for T { … }`. The useful overrides
  are `get_methods`, `collect_repr`, `equals`, `write_hash`, `compare`, `get_attr`, `dir_attr`,
  `has_attr`, `invoke` (callable), `export_as`, `set_attr`, `at`, `iterate_collect` and
  `length`. Example: `spike::Label`.
- **Complex value** (holds `Value<'v>` or interior mutability): write an unfrozen type
  `Foo<'v>` deriving `Trace`, and a frozen type `FrozenFoo` holding `FrozenValue`. Implement
  `Freeze for Foo<'v> { type Frozen = FrozenFoo }` and `AllocValue` via `heap.alloc_complex`.
  Give `FrozenFoo` its own `StarlarkValue` impl with `type Canonical = Foo<'v>`. The derive
  `Freeze` and the `FooGen<V: ValueLike>` + `starlark_complex_value!` pattern are shorter when
  both sides share code. Examples: `spike::Depset`, `ProviderInstance`, `harness::TestStruct`.
- Unfrozen values must be `Send` up to their `Value`s (a `RefCell` is fine, an `Rc` is not).
  Frozen values must be `Send + Sync`.
- **Never-frozen value** (Bazel `ctx`): use `heap.alloc_complex_no_freeze`. Freezing a module that
  still references it fails (tested), which matches Bazel's rule that ctx must not escape
  analysis.
- `#[starlark_value(type = "ctx", UnpackValue, StarlarkTypeRepr)]` generates `&T: UnpackValue`
  for complex types used as a method's `this`.
- The type name is the static `type = "…"` string, so a value cannot choose its type name at
  runtime. Java's `Starlark.type` is also per class (`Starlark.classType`), and Bazel's struct and
  provider instances both report `struct` (`StructApi` `@StarlarkBuiltin(name = "struct")`), so
  this matches. Two type names need two Rust types (as for ScriptTest's `struct` and
  `mutablestruct`).
- **`export_as(&self, name, eval)`** runs when a top-level assignment binds the value, which is
  Bazel's post-assign `StarlarkExportable.export`. `spike::Provider` uses it to set
  `(defining .bzl label, name)` and reads the .bzl label from `Evaluator::extra`. The key
  survives freezing (tested).

### 2.4 Modules, freezing, threads

- **New in 0.14:** a `Module<'v>` only exists inside `Module::with_temp_heap(|module| …)`. The
  heap lifetime `'v` is invariant and branded, so values cannot escape the closure. What escapes
  is `module.freeze() -> FrozenModule` or plain Rust data.
- `FrozenModule` is `Clone + Send + Sync` (two `Arc`s). Use `get(name) -> OwnedFrozenValue` for
  public names (it errors for `_private` names), and `names()` and `extra_value()`.
- `OwnedFrozenValue` / `OwnedFrozenValueTyped<T>` keep the heap alive. `downcast::<T>()` works
  for simple types.
- **Gap:** `OwnedFrozenValue::owned_value(&FrozenHeap)` cannot produce a `Value<'v>` for a
  branded module (the lifetime is the heap borrow's, not `'v`). Blaze needs one audited helper:
  `Value::new_frozen(unsafe { v.owned_frozen_value(module.frozen_heap()) })`
  (`spike::frozen_into`). The safe alternative is `module.import_public_symbols(&frozen_module)`
  followed by `module.get(name)`, which only works for whole modules.
- GC: the evaluator collects garbage between statements unless `disable_gc()` is called. Values
  referenced only from Rust locals across statements are not roots, so natives must not stash
  `Value`s in Rust state.

### 2.5 Evaluation, loads, context, print

- Evaluate a module with `Evaluator::new(&module)`, `eval.set_loader(&dyn FileLoader)`,
  `eval.extra = Some(&ctx)` (a `&dyn AnyLifetime`, so derive `ProvidesStaticType` on the context
  struct; natives call `eval.extra.and_then(|e| e.downcast_ref::<Ctx>())`), then
  `eval.eval_module(ast, &globals)`. `extra_mut` also exists. Call a function with
  `eval.eval_function(f, &[pos], &[("k", v)])`.
- **Loads:** `FileLoader::load(&self, path: &str) -> Result<FrozenModule>` receives the raw
  string from `load("…")`. Bazel resolves every load label before executing a file
  (BzlLoadFunction), so Blaze pre-scans `ast.loads()`, resolves the labels (with repo mapping),
  gets the `FrozenModule`s from Skyframe and hands the evaluator a map keyed by the raw string
  (`spike::MapLoader`, or `ReturnFileLoader`). With `enable_load_reexport = false`, loaded names
  are not re-exported and `_private` names cannot be loaded (tested).
- **Per-evaluation context:** `Evaluator::extra` carries what Bazel puts in StarlarkThread
  thread-locals (`BazelStarlarkContext`, `Package.Builder`, `RuleContext`, repository ctx).
  Natives check the phase there, the way Bazel checks whether a thread-local is present
  (`spike::EvalContext`, `rule_target` test).
- **print:** the stock `PrintHandler::println(&str)` gets no location and no `sep`. Blaze
  overrides the `print` global with its own native (`spike_natives::print`). It reads
  `eval.call_stack_top_location()` and emits `DEBUG: <file>:<line>:<col>: <msg>`, where col is the
  call's `(`. This was verified against the oracle (`DEBUG: …/BUILD.bzl:2:10: analyzing|t`).
- **fail:** the stock `fail(*args)` has no `msg=`, `attr=` or `sep=`, and its message is
  `ErrorKind::Fail(" a b")` (leading space). Blaze overrides it with
  `fail(*args, msg=None, attr=None, sep=" ")`. Java: `MethodLibrary.fail`.
- **Limits:**
  - `set_max_callstack_size(n)`: the default is **50 frames**, natives included
    ("Starlark call stack overflow"). Bazel has no fixed depth, so set it to a few hundred and run
    evaluations on threads with large native stacks.
  - `set_max_tick_count`, `set_max_heap_size`, `set_check_cancelled` (use the last for Ctrl-C and
    Skyframe interruption).
  - There is no step limit unless `set_max_tick_count` is set.
  - Recursion is **allowed** (Bazel forbids it).
- **Typing:** `Dialect.enable_types` and `eval.enable_static_typechecking(true)`. Leave both off:
  Bazel 9.3.2 does not type-check by default.
- **Profiling/coverage:** `enable_profile(&ProfileMode)` and `coverage()`. These may later back
  `--starlark_cpu_profile`.

### 2.6 Errors and call stacks

`starlark::Error` has `kind()`, `span() -> Option<&FileSpan>` (the failing expression) and
`call_stack() -> &CallStack { frames: Vec<Frame { name, location: Option<FileSpan> }> }`. Each
frame's `location` is the call site in the caller. `without_diagnostic()` gives the bare message.
`traceback::render_bazel` maps this onto `EvalException.formatCallStack`:

- Java entry 0 is `<toplevel>` at `frames[0].location`. Entry i is `frames[i-1].name` at
  `frames[i].location`.
- If the error's span equals the last frame's call span (or the error has no span), the innermost
  callee is a builtin: Bazel prints `Error in <name>: msg`. Otherwise the innermost def is listed at
  `error.span()` and Bazel prints `Error: msg`.
- The column of a call is that of its `(` (Java `CallExpression.getLparenLocation`). It is
  recovered from the call expression's source text.

Oracle check (`tests/spike.rs::e_fail_renders_bazel_traceback`, compared against Bazel 9.3.2 on
the same files): the output is byte-identical, including columns. Two remaining differences:

- For non-call errors Bazel points at the operator (`x + 1` gives column 14), while starlark-rust
  spans start at the left operand (column 12). Fix: walk the AST to map an `Op` span to its
  operator position.
- A `Starlark call stack overflow` error has no span and renders as `Error in f:`. Special-case
  the `StackOverflow` kind.

Message texts are starlark-rust's (for example "Operation `+` not supported for types `string`
and `int`" where Bazel says "unsupported binary operation: string + int").

## 3. Conformance: Bazel's ScriptTest on starlark-rust

The harness is `crates/starlark_conformance`. Run it with
`scripts/cargo-capped run -p starlark_conformance -- run -v`; `help` prints usage. The test
`tests/script_test.rs` is a ratchet: it fails if the totals drop. ScriptTest.java semantics are
kept:

- Chunks are split on `\n---\n`, each runs in a fresh module, and line numbers are preserved.
- A `### regex` expectation must match the chunk's error (Java `find`; the Rust `regex` crate
  compiled every pattern).
- `assert_`, `assert_eq` and `assert_fails` report and continue. Their state travels in
  `Evaluator::extra`.
- The harness also provides `struct`, `mutablestruct`, `int_mul_slow`, `_utf8_byte_strings=False`
  and a `json` shim (encode via `Value::to_json`, decode via serde_json, no `indent`).

Because one unexpected error aborts the rest of a chunk, every aborted chunk is re-run statement
by statement in one module ("resilient"). The category counts below come from that re-run. Four
assertions are blanked as unsafe (section 3.2, items 9 and 10).

| file | chunks pass/total | asserts pass/run (in source) | resilient asserts pass/run | missing-builtin | error-text | semantic | dialect-syntax | harness |
|---|---|---|---|---|---|---|---|---|
| all_any.star | 0/1 | 23/27 (27) | 23/27 |  | 4 |  |  |  |
| and_or_not.star | 1/1 | 31/31 (31) | 31/31 |  |  |  |  |  |
| assign.star | 1/1 | 1/1 (1) | 1/1 |  |  |  |  |  |
| bench_*.star (6 files) | 6/6 | 0/0 | 0/0 |  |  |  |  |  |
| comprehension.star | 0/1 | 3/4 (4) | 3/4 |  | 1 |  |  |  |
| cycles.star | 0/3 | 0/3 (3) | 0/3 |  |  | 3 |  |  |
| dict.star | 0/2 | 47/56 (61) | 48/61 |  | 9 | 2 |  | 3 |
| equality.star | 1/1 | 56/56 (56) | 56/56 |  |  |  |  |  |
| fields.star | 0/2 | 19/23 (23) | 19/23 | 2 | 3 |  |  |  |
| float.star | 0/1 | 103/119 (241) | 178/228 |  | 28 | 35 |  |  |
| function.star | 5/12 | 32/48 (49) | 32/49 |  | 5 | 11 |  | 2 |
| int.star | 0/1 | 1121/1136 (237) | 1121/1136 |  | 10 | 5 |  |  |
| int_constructor.star | 0/1 | 58/82 (82) | 58/82 |  | 21 | 3 |  |  |
| json.star (shim) | 0/1 | 20/22 (126) | 57/106 | 9 | 42 | 20 |  |  |
| list.star | 0/1 | 28/34 (34) | 28/34 |  | 6 | 1 |  |  |
| list_mutation.star | 1/5 | 23/29 (35) | 25/34 | 5 | 3 | 3 |  |  |
| list_slices.star | 0/1 | 69/87 (87) | 69/87 |  | 18 |  |  |  |
| loop.star | 4/22 | 18/18 (18) | 18/18 |  | 18 |  |  |  |
| min_max.star | 0/1 | 60/88 (88) | 60/88 |  | 28 |  |  |  |
| range.star | 0/1 | 14/15 (41) | 22/29 |  | 5 | 14 |  |  |
| reversed.star | 0/1 | 14/16 (12) | 14/16 |  | 2 |  |  |  |
| set.star | 0/1 | 38/59 (140) | 61/131 | 20 | 32 | 24 |  | 14 |
| sorted.star | 0/1 | 24/24 (38) | 25/34 |  | 8 | 5 |  |  |
| string_elems.star | 0/1 | 1/1 (4) | 6/6 |  |  | 1 |  |  |
| string_find.star | 1/1 | 43/43 (43) | 43/43 |  |  |  |  |  |
| string_format.star | 6/18 | 43/43 (43) | 43/43 |  | 12 |  |  |  |
| string_misc.star | 0/1 | 31/31 (158) | 141/154 |  | 13 | 5 |  |  |
| string_partition.star | 0/1 | 22/28 (28) | 22/28 |  | 6 |  |  |  |
| string_slice_index.star | 0/1 | 32/40 (40) | 32/40 |  | 8 |  |  |  |
| string_split.star | 0/1 | 4/4 (41) | 38/39 |  |  | 3 |  |  |
| string_splitlines.star | 1/1 | 17/17 (17) | 17/17 |  |  |  |  |  |
| string_test_characters.star | 1/1 | 43/43 (43) | 43/43 |  |  |  |  |  |
| tuple.star | 0/1 | 22/23 (23) | 22/23 |  | 1 |  |  |  |
| **total (38 files)** | **28/95** | **2060/2251 (1874)** | **2356/2714** | **36** | **283** | **135** | **0** | **19** |

Asserts in loops count once per iteration, so "run" can exceed "in source" (int.star). `run -v`
or `failures` prints all 473 failures with file, line and category.

### 3.1 Categories

- **error-text (283).** An error is raised, but with starlark-rust's wording. Examples:
  - "Operation `X` not supported on type `Y`" (58) and "…for types" (39) where Bazel says
    "unsupported binary operation" or "type 'int' is not iterable"
  - "Index `N` is out of bound" (25)
  - "Type of parameter … doesn't match" (14)
  - int() parse messages (11)
  - "Unpacked N values but expected M" (10)
  - serde_json parse messages in the json shim (≈20)

  decisions.md accepts different Starlark error text. Bazel error text only reaches users on
  failing builds.
- **missing-builtin (36).** `set.isdisjoint`, `intersection_update`, `difference_update` and
  `symmetric_difference_update` (20 with knock-on asserts). `json.indent` and `encode_indent`
  (shim). Plus "has no field or method" texts for `struct`/`tuple` attributes (the harness counts
  these as missing builtins, but the behaviour is right and only the text differs).
- **harness (19).** `freeze()` (Java shallow freeze, or closing the thread's Mutability) has no
  starlark-rust equivalent. Values freeze only with their whole module. 3 `freeze` calls and 16
  dependent "trying to mutate a frozen …" asserts. Blaze does not need shallow freeze: Bazel
  freezes whole modules, as starlark-rust does.
- **semantic (135)**, the ones that matter, grouped by root cause in section 3.2.
- **dialect-syntax: 0.** Every chunk parses with the Bazel dialect.

### 3.2 Semantic differences (root causes)

| # | difference (oracle-verified where marked) | failures | impact on real builds | fix |
|---|---|---|---|---|
| 1 | **`<<` overflows i32**: `1 << 31 == -2147483648`, `2 << 31 == 0` (oracle: 2147483648, 4294967296). `InlineInt::checked_shl` checks only the shift count (`int_or_big.rs:332`). Also `1 << 520` does not fail (Bazel: "shift count too large"). | int 5, range 14 (via `maxint = (1 << 31) - 1`) | silent wrong ints; rare in BUILD logic | **patch** (`checked_shl` then compare with `>>`); interim: `replace_binary_operators({"<<": …})` (misses `<<=`) |
| 2 | Raw strings: `r"\""` has 1 char (oracle: 2, backslash kept) | json 1 | rare | **patch** lexer |
| 3 | `str(float)` uses `%g`-like formatting for exponents: `1.23e-45` (oracle: `1.2299999999999998e-45`); `1e16` → `1e+16` (Bazel `10000000000000000.0`); `"%d"/"%x"/"%o" % float` unsupported; float/int comparison not exact (`1.23e45 != 1229…033`); int→float overflow not detected | float 35 | float use in BUILD/bzl is rare | **patch** float repr (shortest round-trip, Java `StarlarkFloat` rules), % conversions, exact comparison |
| 4 | Recursion allowed (Bazel: "function 'f' called recursively"); default max depth 50 | function 4 | only on code Bazel rejects | wrapper: check the call stack in a `before_stmt`-style hook is not public, so **patch** (one flag in `def` invoke) |
| 5 | Builtins: `type(len) == "function"` (oracle `builtin_function_or_method`), `str(len) == "len"` (oracle `<built-in function len>`), `[].clear` repr, lambda repr `file.lambda` (Bazel `<function lambda>`), builtins hashable (`len in {}` ok) | function 7, dict 2 | visible in `print`/`repr` output only | **patch** NativeFunction type/repr, def repr |
| 6 | Cycle repr `[...]` vs Bazel `...` | cycles 3 | none | patch or ignore |
| 7 | Set: `x \|= y` / `&=` / `-=` / `^=` rebind instead of mutating in place; `set.update/union/intersection/difference` accept one argument (Bazel: any number); `pop()` removes the last element (Bazel: the first); repr `set([])` vs `set()`; 4 `*_update`/`isdisjoint` methods missing | set 44 | `set` is new; 23 corpus files call `set(` | **patch** set type (port `StarlarkSet` semantics) |
| 8 | Strings: `split/rsplit(sep=, maxsplit=)` named args rejected; `rsplit("")` accepted; `replace(old, new, -1)` rejected (Bazel: negative = all); `elems()` returns an iterator without `len` | string_split 3, string_misc 4, string_elems 1 | named `sep=` is used in the wild | **patch** string methods |
| 9 | **Crashes:** `[1, 2, 3] * (1 << 30)` panics (`AlignedSize must not exceed u32::MAX`); `"abc" * (1 << 30)` allocates 3 GiB and `[].extend(range(2**31-1))` ~16 GiB (Bazel: "excessive repeat"/"excessive capacity"); `json.encode` of a 100,000-deep list overflows the native stack (uncatchable abort; Bazel: "nesting depth limit exceeded") | 4 blanked | user-triggered process abort or OOM | **patch** repeat/extend size limits and repr/json depth guards; meanwhile run each evaluation under `catch_unwind` |
| 10 | `list.pop(-2)` rejected (Bazel: negative index allowed) | list_mutation 2 | plausible in real code | **patch** |
| 11 | `int("0b0")` accepted in base 10, `int("016", 0)` accepted, `int()` returns 0 (Bazel requires x) | int_constructor 3 | none on valid code | patch |
| 12 | `[d1] <= [d1]` compares dicts (Bazel: identical elements short-circuit); `sorted` orders by code point, Bazel by UTF-16 unit | sorted 5 | edge cases | patch list compare; UTF-16 order: decide (see open problems) |
| 13 | json shim: keys must be sorted, NaN and int keys rejected, `range`/`struct` encodable, surrogate handling, `decode(x, default)` positional | json 20 | Blaze implements `json` itself (port `lib/json/Json.java`) | not a starlark-rust issue |
| 14 | `x.a -= 1` on a struct field: Bazel reports the binary-op error at the operator line (expectation location test) | fields 1 (error-text) | none | — |
| 15 | Top-level rebinding `x = 1; x = 2` accepted in .bzl (Bazel: "'x' redeclared at top level"; BUILD allows it) | (tests/dialect.rs) | only on code Bazel rejects | AST pre-check in .bzl compile |

## 4. Dialect diff and fix plan

### 4.1 Grammar and resolver (Bazel 9.3.2 Java vs starlark-rust)

| item | Bazel | starlark-rust | plan |
|---|---|---|---|
| top-level `if`/`for` | .bzl: Resolver error "for loops are not allowed at the top level…"; BUILD: `DotBazelFileSyntaxChecker` message | `enable_top_level_stmt=false` rejects with its own text | .bzl: dialect (text differs); BUILD: parse with it on, then `build_check` (Bazel text) |
| BUILD: def, lambda, for, if, `*args`, `**kwargs` (literal dict only) | `PackageFactory.checkBuildSyntax` → `DotBazelFileSyntaxChecker` | no switch | `build_check::Checker` (ported; exact texts and 1-based positions tested). Also port the glob/subpackages prefetch extraction and the `generator_name` map from the same visitor. |
| load binds file-locally, not re-exported; `_x` not loadable | Resolver | `enable_load_reexport=false` | dialect |
| `load()` must precede other statements (.bzl; `requireLoadStatementsFirst`) | error "load statements must appear before any other statement" | starlark-rust allows loads anywhere at top level | AST pre-check (.bzl and MODULE files) |
| BUILD: `loadBindsGlobally`, `allowToplevelRebinding` | yes | rebinding accepted everywhere; load binds module-locally | rebinding: fine for BUILD; .bzl: AST pre-check "'x' redeclared at top level". For BUILD `load(…,"x"); x = 1`, verify starlark-rust accepts (it binds load names as private module names) |
| `.scl` ASCII-only string literals | lexer option | none | AST pre-check on string literals for .scl |
| keyword-only `*` | yes | dialect flag | on |
| `/` positional-only | no | flag | off |
| f-strings | no | flag | off |
| type annotations | only with `--experimental_starlark_type_syntax` | `enable_types` | off; if the flag is ever needed, `ParseOnly` |
| `while`, `class`, `is`, `**` operator | not in grammar | not in grammar | — |
| recursion | forbidden | allowed | patch (section 3.2 item 4) |
| int: arbitrary precision | yes | yes (i32 inline + BigInt) | shift patch |
| float repr | shortest round-trip (`StarlarkFloat`) | different | patch |
| `%` format: `%d %i %o %x %X %e %f %g %s %r %%`, no width/precision | yes | same set; width rejected too (text differs) | — |
| string methods | `StringModule` (31 methods) | all present; named args and negatives differ | patch |
| `print` | `DEBUG: file:line:col: msg`, `sep=` | handler gets text only | Blaze native (done in spike) |
| `fail(*args, msg, attr, sep)` | yes | `fail(*args)` | Blaze native |
| `struct`, `json`, `depset`, `Label`, `select`, providers, `native.*` | Bazel natives | n/a | Blaze natives |

### 4.2 What Blaze wraps (no crate change)

- **Globals:** start from `standard + SetType`, keep only `BAZEL_UNIVERSE`, and drop `bytes`,
  `chr` and `ord`. Override `print` and `fail`. Add the Bazel predeclared names per environment
  (BUILD, .bzl, MODULE.bazel, REPO.bazel, `@_builtins`, repository rule and module extension).
- **AST pre-checks before `eval_module`:**
  - BUILD syntax checker, plus glob prefetch patterns and generator names.
  - .bzl: load-first, top-level redeclaration, and the .scl ASCII check.
  - Bazel's `--incompatible_*` static checks, if any matter.
- **Error rendering:** `render_bazel`, with the operator-column fix and the stack-overflow case.
- **Panic containment:** `catch_unwind` around each evaluation, turning a panic into an evaluation
  error. This does not help with native stack overflow, so also run evaluations on big-stack
  threads and set `set_max_callstack_size`.

### 4.3 Patch queue for vendored starlark 0.14.2

Each patch is small, carries a test, and is tracked in `third_party/starlark-rust/PATCHES.md`, in
priority order:

1. int `<<` overflow and the 512-bit shift limit.
2. Repeat, `extend` and list-allocation limits (Bazel "excessive repeat (N * M elements)" and
   "got N for repeat, want value in signed 32-bit range"), plus depth guards in `to_json`/repr.
   Removes the panic, the OOM and the stack-overflow aborts.
3. Raw-string lexing of `\"` and `\'`.
4. Recursion check (a flag on `Evaluator`).
5. String methods: named `sep`/`maxsplit`, negative `replace` count, the `elems()` sequence,
   `rsplit("")` error.
6. `list.pop(negative)`.
7. Set semantics (in-place augmented assignment, variadic methods, `pop` order, repr, the missing
   methods).
8. Float repr and `%` on floats; exact int/float comparison.
9. Builtin `type()` and `repr` names.
10. Error texts, opportunistically, only where a test or the oracle shows them.

The alternative of no fork is possible for the milestone, since brotli and re2 hit none of these
on valid code (the corpus grep found no `split(sep=`, `pop(-`, `<< 31+`, raw `\"` or set
augmented assignment). It leaves items 1, 2 and 9 as silent-wrong or crash risks, though, and
every later fix needs the fork anyway. I recommend the fork now, with an empty patch queue at
first, so the dependency path does not change later. The root should record this in
decisions.md.

Note for every crate in the workspace: `starlark` enables `serde_json/arbitrary_precision`. Cargo
unifies features, so every crate's `serde_json::Number` gets arbitrary precision.

## 5. Spike results (`crates/starlark_conformance/tests/spike.rs`, all passing)

| spike | what it proves | result |
|---|---|---|
| (a) `a_custom_values_survive_freezing` | `Label` (simple value with attributes and methods), `depset` (complex value), `provider()` named by `export_as`, all frozen with a.bzl and read back as `OwnedFrozenValue` | ok |
| (b) `b_loads_a_through_file_loader` | b.bzl loads a.bzl via `MapLoader`. A transitive depset is built and flattened from frozen parts. Loaded names are not re-exported; `_private` is not loadable | ok |
| (c) `c_frozen_function_called_on_another_thread_with_ctx` | b.bzl's frozen `b_impl` (which calls a.bzl's `impl`) runs on a new thread in a fresh `Module`, with a `ctx` value (`alloc_complex_no_freeze`) and `EvalContext` in `Evaluator::extra`. `ctx.run()` records an action, `print` records `DEBUG: a/a.bzl:7:10: analyzing\|t`, and `MyInfo` keeps its key `//a:a.bzl%MyInfo` | ok; 114 µs (release) |
| (c') `c_results_outlive_the_analysis_heap_by_freezing` | Provider instances returned by analysis are frozen into a per-target `FrozenModule` and then used as another target's deps on a different thread (the analysis-cache shape) | ok |
| (c'') `c_freezing_an_unfreezable_ctx_fails` | a leaked `ctx` makes freeze fail | ok |
| (d) `d_parallel_packages_share_frozen_bzl` | 2,000 BUILD files load the same frozen b.bzl and call a macro that calls a loading-phase native through the context. Sequential and rayon results are identical | release: 8.1 ms sequential, 1.6 ms on 16 rayon threads (~4 µs/package) |
| (d') `d_loading_native_rejected_outside_loading_phase` | a phase check through `extra` gives `Error in rule_target: …` | ok |
| (e) `e_fail_renders_bazel_traceback` | `fail()` three frames deep across BUILD → b.bzl → b.bzl | byte-identical to oracle output |
| (e') `e_non_builtin_error_renders_error_prefix` | `Error:` prefix and frames for a non-builtin error | ok (operator column differs, section 2.6) |

The conformance run takes 0.05 s in a debug build. The parse sweep over 8,188 files takes a few
seconds.

## 6. Recommended design: crate `blaze_starlark`

This is layer 4 in `06-bazel-architecture-map.md`. It depends on `starlark` (vendored), `blaze_options`
(BuildLanguageOptions → semantics) and `blaze_util`. Domain values (Label in `blaze_cmdline`,
Depset in `blaze_collect`, providers in `blaze_packages`) implement `StarlarkValue` in their own
crates against the traits here.

```
blaze_starlark/
  semantics.rs   StarlarkSemantics port (BuildLanguageOptions flags → typed struct); Dialect per FileKind
  file_kind.rs   enum FileKind { Build, Bzl, Scl, Prelude, Module, Repo, Workspace, BuiltinsBzl }
  syntax/        build_check.rs (DotBazelFileSyntaxChecker + glob/generator extraction),
                 bzl_check.rs (load-first, redeclaration, .scl ascii); returns Vec<SyntaxError{loc,msg}>
  env.rs         Environment builder: universe filter + per-FileKind predeclared maps; overrides
                 print/fail; registry of natives contributed by other crates
  thread.rs      ThreadContext (the Evaluator::extra payload), Phase, ContextKind accessors
  eval.rs        compile(FileKind, label, bytes) -> Compiled{ast, loads, globs…};
                 exec_module(compiled, loads: LoadMap, env, ctx) -> Result<FrozenModule, EvalError>;
                 call(frozen fn, args, ctx) -> R (fresh Module per call); frozen_into helper (the one unsafe)
  loader.rs      LoadMap: FileLoader over HashMap<raw load string, FrozenModule>
  error.rs       EvalError { message, callstack: Vec<Entry{name, file, line, col}>, kind };
                 render() = EvalException.formatCallStack; SyntaxError list rendering "file:line:col: msg"
  printer.rs     Bazel repr/str/debugPrint for Blaze-owned types (Printer.java), print → event reporter
  json.rs        port of net/starlark/java/lib/json/Json.java as a namespace
  structs.rs     Bazel `struct` (StructImpl/StarlarkInfo base), to_json removed per 9.x semantics
  types.rs       helper traits/macros for simple/complex/no-freeze values, unpack helpers matching @Param
```

### 6.1 Contexts per phase (the `Evaluator::extra` payload)

One `ThreadContext` struct, matching Bazel's `BazelStarlarkContext` plus the phase-specific
thread-locals:

```rust
#[derive(ProvidesStaticType)]
pub struct ThreadContext<'a> {
    pub phase: Phase,                       // Loading | BzlLoad | Analysis | Repository | ModuleExtension | Builtins | Module
    pub file_label: &'a Label,              // provider/rule export keys, error locations
    pub semantics: &'a StarlarkSemantics,
    pub repo_mapping: &'a RepositoryMapping,
    pub events: &'a dyn EventHandler,       // print → DEBUG event, warnings
    pub symbol_generator: SymbolGenerator,  // Bazel's per-thread symbol ids (provider/rule identity)
    pub kind: ContextKind<'a>,
}
pub enum ContextKind<'a> {
    Bzl,                                                   // top-level .bzl eval: rule()/provider()/aspect() allowed
    Package(RefCell<&'a mut PackageBuilder>),              // BUILD + macros: native rules, glob, package()
    Analysis(&'a RuleContextState),                        // rule impl: ctx actions, providers
    Repository(&'a RepositoryCtxState),                    // repository_ctx (download/execute are blocking Rust calls)
    ModuleExtension(&'a ModuleCtxState),
    Module(RefCell<&'a mut ModuleFileBuilder>),            // MODULE.bazel
}
```

`ThreadContext::get(eval)` returns an error with Bazel's text when a native is called in the wrong
phase (for example "… can only be called during the loading phase").

### 6.2 Threading model

- One `Module::with_temp_heap` per evaluation unit, run on the Skyframe worker thread that owns
  that SkyFunction call. Units: a BUILD file, a .bzl top level, one rule implementation call, one
  repository rule invocation, one extension evaluation. Never hold a `Module` across a Skyframe
  dependency request: starlark-rust cannot suspend an evaluation (see 06's note). Evaluate only
  after every dependency is ready, which is how Bazel uses BZL_LOAD values too.
- Results crossing threads or Skyframe nodes are `FrozenModule` / `OwnedFrozenValue` (Send+Sync),
  or plain Rust data extracted before the heap drops. Analysis freezes the returned providers into
  a per-target frozen module (spike c').
- Repository rules and module extensions do blocking I/O inside natives (download, execute). Run
  them on a dedicated big-stack thread pool so Skyframe workers are not starved.
- Thread stack: ≥16 MiB for evaluation threads. Set `set_max_callstack_size` to a few hundred,
  `set_check_cancelled` to the Skyframe interrupt flag, and wrap each evaluation in `catch_unwind`.

### 6.3 Registering natives

- Each Java `@StarlarkMethod` group becomes one `#[starlark_module]` function in the crate that
  ports the Java class, marked `// Java: … Class.method`. `blaze_starlark::env` composes them per
  `FileKind` (Bazel's `BazelStarlarkEnvironment`: BUILD env, bzl env, `native` namespace = BUILD
  natives, `@_builtins` injection).
- Rule classes (`rule()` results, native rules) are values with `invoke`. Calling one with
  `ContextKind::Package` adds a target. Their identity is set by `export_as`.
- `Globals` are built once per (environment, semantics) and shared as `Arc` (they are Send+Sync).
- Parameters follow Java `@Param`s: positional/named flags, defaults, `allowedTypes` checks with
  Bazel's error text where tests need it.

### 6.4 Errors

`EvalError` keeps a structured stack (`name, file, line, col`), converted from `starlark::Error`
by the logic in `traceback::render_bazel` (lparen columns; builtin leaf → `Error in <fn>: `). The
message text stays starlark-rust's unless patched. Events print it as Bazel does:
`ERROR: <file>:<line>:<col>: Traceback (most recent call last):\n…`. Syntax errors render as
`file:line:col: message` lists, as `SyntaxError.Exception` does.

## 7. Open problems

- **UTF-16 vs UTF-8 strings.** Bazel's `len`, indexing, ordering and `elems()` are UTF-16
  (Java) unless `INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS` is set. starlark-rust is
  code-point-based for `len` and indexing. ASCII-only builds agree. The root needs to decide
  whether non-ASCII fidelity matters.
- **Operator columns in tracebacks**: done (blaze_starlark `Location::bazel_position` and patch 0028; section 9).
- **Fork decision** (section 4.3) for the root to rule on in decisions.md.
- **Upgrades.** `tests/dialect.rs::deviation_*` pins 9 current deviations. When a patch or an
  upgrade fixes one, its test fails, and the test and this document get updated.
- **Performance** of large real evaluations (rules_cc's ~29k loaded lines) is not measured yet.
  The spike shows ~4 µs per trivial package under rayon.

## 8. Status update (implementation)

- starlark 0.14.2 is vendored in `third_party/starlark-rust` (`[patch.crates-io]`, excluded from
  the workspace). Patches 0001–0004 and 0006 of section 4.3 are applied; see
  `third_party/starlark-rust/PATCHES.md` (one test each in
  `crates/starlark_conformance/tests/patches.rs`). Conformance: 30/95 chunks, 2180 strict and
  2443/2744 resilient assertions (from 28/95, 2060, 2356/2714), with nothing unsafe to run.
- `crates/blaze_starlark` implements section 6: `semantics` (generated from
  BuildLanguageOptions, 65 flags), `file_kind`, `syntax` (DotBazelFileSyntaxChecker with BUILD
  glob/subpackages/generator-name extraction; Resolver rebinding, load-first and .scl rules),
  `env` (universe, Bazel `print`/`fail`, `NativeRegistry`), `thread` (`ThreadContext` with an
  opaque payload), `eval` (compile, exec_module, call_frozen, frozen_into, evaluation threads,
  cancellation), `loader`, `error` (Bazel tracebacks incl. operator/`[`/`.`/`(` columns), `printer`,
  `json` (Json.java port), `structs` (`struct`, StarlarkInfo base), `depset` (Depset.java + the
  NestedSet structure rules, convertible to `blaze_collect::NestedSet`) and `label`.
- Crate graph: blaze_starlark depends on blaze_cmdline (Label) and blaze_collect (Order,
  NestedSet), i.e. it sits above them, not below as in section 5.2 of 06-bazel-architecture-map.md.
  Neither needs Starlark itself.

## 9. Patch queue status

State of the working tree on 2026-11-18 (on top of commit `1a68622`, not committed yet). Every
entry is in `third_party/starlark-rust/PATCHES.md` (with the Java file:line it follows), every
changed line carries `BLAZE PATCH NNNN`, `patches/blaze.diff` is regenerated, and each patch has
a `patch_NNNN_*` test in `crates/starlark_conformance/tests/patches.rs`.

**Applied:** 0001–0009, 0011–0031. **Queued:** 0010 (error texts; only the parts in 0008, 0023
and 0025 are applied). Added in this round:

| # | what |
|---|---|
| 0005 (rest) | `split`/`rsplit`: `sep` required and nameable, named `maxsplit`, "Empty separator"; negative `replace` count |
| 0007 | set semantics: in-place `\|=` `&=` `^=` `-=`, variadic methods, `isdisjoint` and the `*_update` methods, `pop` of the first element, `set()` repr, Java's argument errors |
| 0008 | float `str`/`repr`/`%e`/`%f`/`%g` with Java's digits, `%d %o %x %X` of floats (and Java's two's complement), exact int/float comparison, "int too large to convert to float", division texts |
| 0009 | `builtin_function_or_method`, `<built-in function len>`, `<built-in method m of T value>`, `<function f[ from M]>`; builtins unhashable |
| 0026 | `StarlarkValue::get_attr_fallible` / `has_attr_fallible` (errors and the evaluator); `getattr` default only for a missing field |
| 0027 | `StarlarkValue::at_with_eval` / `is_in_with_eval` |
| 0028 | Java's error locations (oracle-checked, 48 of 51 cases), no call inlining, `int()` errors not syntax errors |
| 0029 | `starlark::eval::resolve_errors`: every unresolved name |
| 0030 | var statements `x: T` |
| 0031 | cycles print `...` |

blaze_starlark (not patches): `EvalError::from_starlark` drops the `File "<builtin>", in
<toplevel>` entry of a function called from native code; the `not in` operator is located at its
`in`; `eval::resolve` (all unresolved names as one syntax error list); the var statement in the
top-level binding checks; `structs::is_immutable` and `json` follow the new type names.
Location oracle: `scripts/starlark-locations record` writes
`crates/blaze_starlark/tests/data/locations/*.oracle.txt`, compared by
`traceback_locations_test`.

**Conformance** (Bazel ScriptTest, `starlark_conformance run`; strict / resilient as in section
3):

| tree | chunks | strict asserts | resilient asserts | categories (missing-builtin / error-text / semantic / harness) |
|---|---|---|---|---|
| before this round (HEAD `1a68622`) | 31/95 | 2186/2371 | 2444/2745 | 27 / 240 / 98 / 19 |
| after (this tree) | 37/95 | 2494/2658 | 2567/2773 | 7 / 207 / 18 / 19 |

The ratchet in `tests/script_test.rs` is now 37 chunks and 2494 assertions.

**Remaining known deviations:**

- Error texts (queued 0010): 207 ScriptTest assertions, e.g. "Operation `+` not supported for
  types ..." (Java "unsupported binary operation: int + string"), "Index `3` is out of bound"
  (Java "index out of range (index is 3, but sequence has 3 elements)"), "Value of type `list`
  is not hashable" ("unhashable type: 'list'"), parameter type and arity texts, `int()` parse
  texts, undefined names ("Variable `x` not found"; Java "name 'x' is not defined").
- Semantic (18): `int()` base inference (`int("016", 0)`, `int()` without argument), range
  lengths beyond 32 bits ("Integer overflow"), `sorted` of dicts/None (starlark-rust compares
  them; Java identical-element short cut), UTF-16 ordering of strings, json's surrogate handling.
- Harness (19): Java's shallow `freeze()` has no starlark-rust equivalent.
- Locations: calling a non-callable value, `f(*x)` / `f(**x)` with a bad `x` (pinned in
  `traceback_locations_test::KNOWN_DEVIATIONS`).
- A def's repr names ` from <module>` only once its module is frozen (Java prints the client data
  while the module still executes too).
- `eval_module` itself still stops at the first unresolved name; callers that want Java's list
  call `blaze_starlark::eval::resolve` first.
- GC tracing, freezing and equality of very deep values recurse on the native stack (0002).
