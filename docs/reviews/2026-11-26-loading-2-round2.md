# Review, round 2: loading-2

Date 2026-11-26. Reviewer: Fable 5.2, read-only. Commit under review: **5f68157**.
Re-check of `2026-11-25-loading-2.md` (STOP at e8b79c0, blocking F1) against
`2026-11-25-loading-2-response.md`.

Verdict: **PASS**, bound to 5f68157. F1 is fixed. Three S2 items are open (R1, R2, R3 below);
R1 answers the question "is the regex loud rule complete" with no.

No Blaze or Bazel process was started (`root-suite-5f68157` was active throughout). No Cargo.
Two things were run: the system JDK (25.0.5) on one small file, and `rg` 15.2.0 (it matches with
the `regex` crate) on the same nine patterns. Both are in
`~/.cache/blaze/work/review-loading-2-round2/P.java` and section R1. The oracle probes the
response cites (c01-c13, p20-p50, q60-q104, s01, s02) were not re-run; where I rely on them I
say so.

## Scope

Read: `git diff e8b79c0 5f68157` of `third_party/starlark-rust/starlark/src` (all seven changed
source files), `PATCHES.md` 0057-0060, `crates/blaze_starlark/src` (eval, lexer, parser,
resolver), `crates/blaze_util/src/java_regex.rs`, `scripts/java-param-types/{run,ParamTypes.java}`,
the table `values/java_param_types.tsv` (by script: ambiguous pairs, duplicate keys, receivers),
`crates/blaze_packages` (`implicit_outputs_function.rs`, `macro_function.rs`,
`rule_class_functions.rs`), `bzl_load.rs`; the Java they port (`Eval.execLoad`,
`Module.getGlobal/getGlobals`, `ParamDescriptor`, `StarlarkRuleClassFunctions.macro`,
`ImplicitOutputsFunction`, `StarlarkCallbackHelper`, `Rule.populateImplicitOutputFiles`).

Not read: `patches/blaze.diff` itself (the vendored sources were read instead), the new tests in
`patches.rs` beyond 0058, the 267 changed rows of `rust_error_texts_allowed.tsv` (only the count
of REVIEW rows and the rows of F1 and 0060), `syntax_test.rs`, the 14,000 generated regex rows,
everything round 1 listed as not read.

## Round-1 findings

| finding | state at 5f68157 |
|---|---|
| F1 (S1, blocking) | **fixed**. `load_symbol` (`environment/modules.rs:685`) follows `Eval.execLoad` (Eval.java:261-272): `java_get_global` (`:336`) returns no value for a name the loaded file bound by `load` or never assigned, the text is Java's with the module name as written, the suggestion runs over `java_globals` (assigned, not load-bound, binding order), and the error is located one char into the original name's literal (`eval/compiler/module.rs:94`). A private name is still rejected first, with Java's resolver text. tsv row `Eval.java:267` is `matched`. Text and location rest on the response's probes c01-c05. |
| F2 | fixed (`lexer.rs` keeps `DocComments.toString`, `parser.rs:107` prints it). |
| F3 | fixed for the cases of round 1 (key cast, `checkAttributeName`, value cast; parameter types before the flag check). Residue, marked PORT at the site: two ill-typed named arguments are reported in declaration order, Java in call order. |
| F4 | fixed (`SmallMap`, each source fed in its own Java order). |
| F5 | **not complete**: R1. |
| F6 | narrowed, not closed: R2. |
| S3 `compile_with_options` | fixed. |
| S3 parser stack overflow | unchanged, reason accepted (JVM-dependent depth and text). |
| 134 REVIEW rows | no REVIEW row is left (the word occurs only in the file's header comment); the F1 rows are reclassified as host API. The classifications themselves were not re-read. |

## Findings

### S1

None.

### S2

**R1. `java_regex`: a lazy counted repetition returns a different span silently.**

- Rust: `crates/blaze_util/src/java_regex.rs:556` `find_span_issue` skips every
  `Qtype::Lazy` repetition ("Lazy repetitions try the exit first in both and agree");
  `assertion_loop_issue` needs a zero-width assertion. The pattern is emitted unchanged as
  `(?:body){min,max}?` (`:612-627`).
- Java: `Pattern.LazyLoop.match`: after a non-empty iteration it tries `next`, then one more
  iteration; an empty iteration goes to `next` at once. The crate expands `e{n,m}?` to copies
  of `e`, so an empty first copy followed by a non-empty one is tried before Java's
  "non-empty, then one more". With a finite maximum of 2 or more the two orders reach
  different ends.
- Input (JDK 25.0.5 against the `regex` crate through ripgrep 15.2.0, both run for this
  review):

  | pattern | input | Java `find()` | crate |
  |---|---|---|---|
  | `(?:\|ab\|a){1,2}?(?:b\|c)` | `ababc` | (0,5) | (0,2) |
  | `(?:\|ab\|a){2}?(?:b\|c)` | `ababc` | (0,5) | (0,2) |
  | `(?:\|ab\|a){0,2}?(?:b\|c)` | `ababc` | (0,5) | (0,2) |
  | `(?:\|ab\|a){2,3}?(?:b\|c)` | `ababc` | (0,5) | (0,2) |
  | `(?:\|ab\|a)+?(?:b\|c)` (unbounded) | `ababc` | (0,5) | (0,5) |
  | `(?:a\|ab\|){2}(?:b\|c)` (greedy, empty last) | `ababc` | (0,2) | (0,2) |

  ripgrep's crate version is not Blaze's pinned one; the result follows from how the crate
  expands a bounded repetition, which has not changed. The implementer should confirm with a
  row in the Blaze test.
- Why the 14,000 rows miss it: no body has an empty alternative followed by two non-empty
  alternatives of different length, and the only lazy counted quantifier is `{2,3}?`.
- Masked: the span is used only by `blaze_remote/src/scrubber.rs` (`transformArgument`), and no
  known configuration has such a pattern. `is_match` and `matches` are not affected.
- Fix expected: in `find_span_issue`, treat a lazy repetition with a finite maximum of 2 or
  more like a greedy one (body can match empty before a non-empty string, body can consume);
  add bodies such as `|ab|a` and quantifiers `{1,2}?`, `{2}?`, `{0,2}?` to
  `JavaRegexNullableRows.java`; correct the sentence in `docs/java-regex.md`.
- Otherwise the rule holds as far as I could derive it: greedy loops whose body matches empty
  only as its last choice agree (the crate's "empty, then non-empty" paths are tried after the
  equal "non-empty, then empty" paths); unbounded lazy loops agree; the zero-width leaves the
  translator can emit (`\A`, `\z`, `(?m:^)`, `(?m:$)`, the four `\b` forms) are all in
  `is_zero_width_assertion`; `$` without MULTILINE and `\Z` are `\z` under a guard.

**R2. Parameter-type table: a builtin or parameter that is not in the table still gets the
derived text, silently.**

- Rust: `third_party/starlark-rust/starlark/src/values/unpack.rs:388`
  `java_param_type_error_message`.
- How a row is selected: rows are keyed by (method, parameter). For a method call the row whose
  receiver column equals the receiver's `type()` is taken (`NativeMeth::invoke` passes it). If
  there is no such row, or the call is a function call (no receiver), the text is used only
  when every row of that (method, parameter) agrees. Otherwise, and for `-` (Java `Object`),
  and for any (name, parameter) missing from the table, it falls back to `java_allowed_types`,
  the derivation round 1 called a guess. Nothing fails and no test lists Blaze's builtins
  against the table.
- The table itself is sound: `ParamTypes.java` repeats `ParamDescriptor.of` and
  `getTypeErrorMessage` (ParamDescriptor.java:91-144) with Bazel's `Starlark.classType`; 1,213
  rows, no duplicate (receiver, method, parameter), 8 pairs that differ by receiver (`add`,
  `add_joined` x2, `run`, `index` x2, `get`, `configure_features`). It does not call
  `getTypeErrorMessage` itself, as the response says it does; the copy is literal.
- Two concrete gaps: (a) a function call never matches the empty-receiver row, so
  `configure_features(requested_features=)` (rows: empty receiver `list`, `cc_common`
  `sequence`) gets the derived text unless it is called as a method of a value whose `type()`
  is `cc_common`; (b) a Blaze builtin whose parameter is named differently from Java's falls
  back without notice.
- Masked: every probed text (p20-p50, q100, q104) is equal.
- Fix expected: a test that walks the registered globals and methods and requires each typed
  parameter to have a row or an entry in a short reasoned exception list; for a function call,
  take the empty-receiver row first.

**R3. Host callers still accept a name a file only binds by `load`.**

- Rust: `crates/blaze_sky_analysis/src/aspect_function.rs:1232,1240`,
  `crates/blaze_repo_rules/src/repository/repo_definition_function.rs:381`,
  `crates/blaze_repo_rules/src/bzlmod/runnable_extension.rs:245` call `get_any_visibility`,
  which finds load bindings. Their PORT comments say so. Patch 0057 added
  `FrozenModule::java_get_global` for exactly this and nobody calls it outside the vendored
  crate.
- Java: `Module.getGlobal` (Module.java:280) is null for a load binding
  (`AspectFunction.loadAspectFromBzl`, `RepoDefinitionFunction`).
- Input: `b.bzl` contains only `load(":a.bzl", "my_rule")`; MODULE.bazel has
  `use_repo_rule("//:b.bzl", "my_rule")`. Bazel: "repository rule //:b.bzl%my_rule does not
  exist (no such symbol in that file)". Blaze uses the rule. Same for
  `--aspects=//:b.bzl%my_aspect` ("my_aspect is not exported from //:b.bzl" in Bazel).
- Not part of this fixer's files; older than this round. Fix expected (owners of those crates):
  replace the three lookups with `java_get_global`.

### S3

- `--experimental_starlark_type_checking`, parser stack overflow: as round 1.
- N1 and N2 of the response (location of a parameter type error inside an assignment; `len(1,
  2)` reports the arity before the type) are real differences in error output, found and
  reported by the fixer and not fixed. They need an entry among the known deviations in
  `docs/STATUS.md` if they are not there yet.

### S4

- `ParamTypes.java` and the response describe the table as printed by
  `ParamDescriptor.getTypeErrorMessage`; it is a copy of that method.

## Checked and found equal

- Patch 0059: `NativeFunction::at` is the unsupported-index error for every builtin; Java has
  no `[]` on a builtin function (text by probe q89).
- Patch 0060: one constant, 20,000 in debug and release. Java has no limit. See open item 1.
- Implicit-outputs callback (`implicit_outputs_function.rs:478-595`) against
  `ImplicitOutputsFunction.java:87-128` and `StarlarkCallbackHelper.java:59-91`: configurable
  attributes left out, Starlark names, one positional argument per callback parameter, the
  unknown-field message with the sorted "Available attributes" suffix (the
  `IllegalArgumentException` becomes an `EvalException` with the same message,
  EvalException.java:71,89), `Dict.cast` text, template expansion, the invalid-placeholder
  error, print events as DEBUG, the defining thread's semantics kept by `rule()`. The caller
  reports `getMessageWithStack()` (Rule.java:577) and the Rust passes the stacked message.
- `rule(outputs=)` type check: `dict, NoneType, or function`, before the body.
- `bzl_load.rs`: the compile environment now comes from `AutoloadSymbols` for the key's
  repository (BzlCompileFunction.java:151).

## Open items that do not block

1. Patch 0060 assumes comparisons run on a 256 MiB stack. The guard is also on equality and
   hashing (`values/layout/value.rs:960,1512`, `value_not_special.rs:75`), which can run on
   other threads (Skyframe value comparison); 20,000 nested levels there would overflow a
   default stack instead of failing cleanly. Needs adversarial input.
2. Implicit-outputs callback: two attributes with the same Starlark name (`$x` and `:x`): Java's
   `HashMap.put` keeps the last, the Rust lookup takes the first, and the "Available
   attributes" list would print the name twice.
3. `get_attribute_type` returning `None` skips the attribute; Java would not skip.
4. F3's residue (order of two ill-typed named arguments) applies to every hand-checked builtin.

## Verdict

**PASS**, bound to 5f68157. No blocking finding. R1, R2 and R3 are S2 and are expected to be
fixed (R1 and R2 by the loading fixer, R3 by the owners of the two crates) or answered with
Java evidence in the next batch.
