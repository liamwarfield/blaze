# Response to the loading-2 review

Date 2026-11-25. Answers `2026-11-25-loading-2.md` (verdict STOP at e8b79c0; blocking F1; S2
F2-F6; the S3 items). Written by the implementer of task `loading-fix-1`.

Tree: HEAD `aa55920` plus uncommitted changes (this task's and other agents'). Nothing is
committed. Every number below is from that tree.

Evidence: oracle probes, one package per case in one workspace
(`~/.cache/blaze/work/loading-fix-1/ws/`, 102 packages), `build //PKG:all` with Bazel 9.3.2 and
with Blaze (`target/debug/blaze` copied to `~/.cache/blaze/work/loading-fix-1/blaze-bin`), each
with its own `--output_user_root`, `--nohome_rc --nosystem_rc`, `env -i HOME PATH`. Compared:
exit code and every `ERROR`/`Traceback`/tab-indented traceback/`Error` line of stderr, workspace
path normalised (`run-probes.sh`, `cmp.sh` in the same directory; outputs in `oracle/`,
`blaze/`).

## Summary

| finding | state | where | test / oracle |
|---|---|---|---|
| F1 load of a missing or load-bound symbol | fixed (patch 0057) | vendored `environment/modules.rs`, `environment.rs`, `eval/compiler/module.rs` | `patch_0057_load_missing_symbol`; probes c01-c05 equal (text, did-you-mean, location) |
| F2 doc comment token text | fixed | `syntax/java/lexer.rs`, `parser.rs` | `doc_comment_token_in_syntax_error`; probes c06-c08 equal |
| F3 `macro()` check order | fixed (one residue, below) | `starlark_defs/macro_function.rs` | probes c09-c13: texts equal; c10-c12 differ in the traceback column (finding N1) |
| F4 nondeterministic suggestion ties | fixed | `syntax/java/resolver.rs` | existing resolver tests |
| F5 regex `find()` spans with nullable repeated bodies | fixed by LOUD, plus a second case the probes found | `blaze_util/src/java_regex.rs` | `nullable_repetitions_follow_java_or_are_loud`: 14,000 JDK 25 rows, 0 silent differences |
| F6 guessed "want" of parameter type errors | fixed (patch 0058) | vendored `values/unpack.rs`, `values/types/function.rs`, `values/java_param_types.tsv` | `patch_0058_java_param_types`; probes p20-p50, q95-q104: every text equal |
| S3 parser stack overflow | not fixed, reason below | | |
| S3 `compile_with_options` error list | fixed | `blaze_starlark/src/eval.rs` | `bzl_parse_errors_then_resolver_errors`; probes s01, s02 equal |
| S3 134 REVIEW texts | all classified or ported | guard list, patches 0057, 0059, 0060 | `error_text_guard` passes with 0 REVIEW rows |

## F1

`load_symbol` now follows `Eval.execLoad` (Eval.java:261): the loaded module's global (any
visibility; not a name that module bound by its own `load`), else
`file 'M' does not contain symbol 'S'` + `SpellChecker.didYouMean(S, module.getGlobals())`, `M`
as written in the load statement, located one char after the start of the original name's string
literal (Parser.java:1442/1460). Load bindings are recorded per module
(`Module::add_load_bound_name`, kept in `FrozenModuleData::load_bound`); `FrozenModule::java_globals`
is `Module.getGlobals().keySet()` (binding order, assigned names, without load bindings) and
`FrozenModule::java_get_global` is `Module.getGlobal` (public, for the host callers whose
comments say starlark-rust's private visibility also covers load bindings: `aspect_function.rs`,
`repo_definition_function.rs`; not changed by me). `CannotImportPrivateSymbol` carries Java's
text (Resolver.java:610) although the Java front end reports it first.

Oracle (all equal, Blaze and Bazel):

- c01 BUILD `load(":a.bzl", "fooo")`: `file ':a.bzl' does not contain symbol 'fooo' (did you mean 'foo'?)` at 1:17.
- c02 the load-bound case of the review: `file ':b.bzl' does not contain symbol 'foo' (did you mean 'food'?)`.
- c03 the same from a .bzl with an alias (`x = "fooo"`): 1:21.
- c04 a private global as candidate: `(did you mean '_bar'?)`.
- c05 no suggestion, odd spacing: `... symbol 'zzzzzz'` at 1:21.

`docs/starlark-errors.tsv` row Eval.java:267 is `matched`; the three guard rows are reclassified
(host API `FrozenModule::get*`) and the private-symbol row is gone (now Java's text).

## F2

The Java lexer port keeps each doc comment's text (`DocComments.toString`, lines joined by
newlines) in the token value; `token_string` prints it. c06 `x = #: doc` → `syntax error at
'#: doc': expected expression` (1:5); c07 `if x #: doc` → `expected :` (2:10); c08 a doc comment
block before a stray `)`. All equal.

## F3

`macro()` checks its parameters' types first, in declaration order (implementation, attrs,
inherit_attrs with Java's `rule, macro, string, or NoneType`, finalizer, doc), then the flag and
`BzlInitThreadContext`; in the attrs loop the key cast, `checkAttributeName`, then the value cast.
Texts equal on c09-c13. PORT (written at the site): Java checks named arguments in the order of
the call, Blaze in the order of declaration; they differ only when two arguments are ill-typed.

## F4

`Block::bindings` and `toplevel` keep insertion order (`SmallMap`). `get_all_symbols` feeds each
source in its own Java iteration order (`java_hash_set_order` of that map's insertion order, the
candidates' HashSet likewise) into the final `java_hash_set_order`, which is what Java's
`HashSet.addAll` of `HashMap.keySet()`s does.

## F5

Confirmed and wider than reported. `tests/data/JavaRegexNullableRows.java` generates 14,000 rows
(35 nullable bodies x 10 quantifiers x 4 suffixes x 10 inputs) from JDK 25; the new test runs
every row through `Pattern` and requires agreement or a loud failure. Two rules, both from
`Pattern.Loop.match` (an empty iteration ends the loop, also below the minimum count):

1. A greedy repetition (`m >= 2`) whose body can match empty before a non-empty string and can
   consume input: `Pattern::find` (the span) and `translate`/`compile` are `unported!`;
   `is_match`/`matches` stay exact. JDK: `(?:a*|b)*` on `abab` is (0,1); the crate (0,4).
2. New: a repetition whose nullable, consuming body contains a zero-width assertion
   (`(?:a|\b){2}b`): Java never follows an empty iteration with a non-empty one, the crate does,
   and where the assertion holds at one position and not the next even the boolean differs
   (JDK: `(?:a|\b){2}b` does not match `abc`; the crate does). `unported!` in every mode.

Result: 8,400 rows exact, 3,360 loud at `find()`, 2,240 loud at compile (these include the
existing possessive/atomic rules), 0 silent differences. Rule 1 is conservative: 3,265 of its
3,360 loud rows would have agreed. `docs/java-regex.md` states the exception and the new rows.

## F6

`scripts/java-param-types/run` (new; `ParamTypes.java` runs Bazel's own classes from the
oracle's `A-server.jar` on the system JDK) prints `ParamDescriptor.getTypeErrorMessage` for every
`@StarlarkMethod` parameter (1,213 rows: receiver `@StarlarkBuiltin` name, method, parameter,
text). The table is `values/java_param_types.tsv` in the vendored crate; patch 0058 takes the
receiver's entry (methods pass their receiver's `type()`), else the one text all of Bazel's
methods of that name agree on (8 method/parameter pairs differ by receiver), else 0052's
derivation. Oracle: p20-p50 (glob, select, depset, join, print, split, list/string `index`,
getattr, range, replace, enumerate, splitlines, startswith, reversed, any, hash, int, fail,
extend, update, sorted, attr.string, attr.label_list, rule, provider, json.encode_indent,
native.existing_rule), q100, q104: every message equal. Blaze builtins that take `Value` and
check by hand still need their own Java check: `rule(outputs=)` was one (root's acceptance case
`outputs_test#test_output_select_error`, now equal).

## S3

- Parser stack overflow (Parser.java:1383): not changed. Java's message ends with the JVM stack
  trace, and the depth at which it fires is the JVM thread's stack size; neither can be
  reproduced. Blaze's Java-parser port recurses on the evaluation stack; the row stays
  `not-triggerable` with that reason.
- `compile_with_options`: a .bzl file's parse errors are followed by the resolver's errors on the
  recovered tree, as in `compile_and_resolve`; the resolver module used there now reports the
  recovery's invalid identifiers ("contains syntax errors", Resolver.java:946). Oracle s01
  (`x = (1 +)` plus a redeclaration and an undefined name): Bazel prints exactly the syntax error
  and `1:5: contains syntax errors`; so does Blaze, through both paths. s02 (BUILD): parse error
  alone, equal.
- The 134 REVIEW rows: see below.

## The 134 REVIEW rows (task part B)

Every row of `crates/starlark_conformance/tests/data/rust_error_texts_allowed.tsv` now carries a
reason; none says REVIEW. Of the 134: 1 removed (the private-symbol text is now Java's, patch
0057); 10 are Java's text with an operand filled in (the guard cannot match a literal without
placeholders); 11 are wrappers passing on a message built elsewhere; 44 internal (host API such as
`FrozenModule::get`, which is where F1's three old texts now live; profiling; serialization; test
loaders; internal errors); 36 dead or replaced code (variants with no constructor after patches
0050-0052, `format_upstream`, `percent_upstream`, the shift and dict paths), each checked by an
oracle probe where the construct is reachable from Starlark (q60-q82, q90-q99); 9 syntax texts the
Java front end reports first; 18 features Bazel lacks or Blaze does not set (`chr`, `ord`,
`partial`, evaluated type expressions, f-strings, tick and heap limits); 5 reachable only past a
limit or as a fallback (call depth over 1000 frames with recursion forbidden; nesting over 20,000
after patch 0060; host argument binding in `CtxSuper`, open; the two operator fallbacks outside
patch 0051's table). Probes found two more deviations behind these rows, fixed: `list[int]`
evaluated in Blaze (patch 0059; Bazel's "type 'builtin_function_or_method' has no operator
[](...)", q89 equal) and the comparison depth guard (patch 0060; Bazel compares 5000-deep lists,
q87 equal; at 5000 `<` crashes Bazel, q88, Blaze answers).

## New findings from the probes (not fixed)

- N1 (S2, location only): for a builtin call with named arguments in an assignment, Java locates
  a parameter type error raised while binding arguments at the statement's `=` (the error leaves
  `evalCall` without `setErrorLocation`; `execAssignment` sets it), Blaze at the `(`. 16 probes
  (c10-c12, p20, p22, p23, p32, p42-p49, q96, q100); the message and `Error in NAME:` are equal.
  In an expression statement both use the `(` (p25, p39 equal). Fixing it needs a static
  "enclosing error location" per call in the vendored compiler and a way for hand-written Blaze
  builtins to mark their argument errors.
- N2: `len(1, 2)`: Java type-checks the bound positional argument before the arity (q95, "want
  'iterable or string'"); Blaze reports the arity.
- The three traceback-location items of STATUS (call of a non-callable, `*x`, `**x`): still
  `KNOWN_DEVIATIONS`; q83 shows the first (`Error in 1:` vs `Error:`).

## Verdict request

F1 is fixed and oracle-checked; F2-F6 and the S3 items are fixed or answered above. Please
re-check.
