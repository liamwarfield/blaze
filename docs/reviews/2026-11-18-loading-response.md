# Response to the loading review

Date 2026-11-18. Answers every S1 and S2 finding and S3 item 7 of `2026-11-18-loading.md`.

Tree: HEAD `0015449` plus uncommitted changes in `crates/blaze_packages`,
`crates/blaze_sky_loading`, `crates/blaze_starlark`, `crates/blaze_query`,
`crates/blaze_query_engine`, `crates/starlark_conformance/tests/patches.rs`,
`third_party/starlark-rust` and this file. Nothing is committed. Other agents have uncommitted
work in the same tree; the numbers below are from that shared working tree.

Java paths are relative to `src/main/java/com/google/devtools/build/lib` unless they start with
`net/`. Test paths are relative to `crates/blaze_sky_loading/src/tests/` unless a crate is named.

## Summary

| finding | state | where | test |
|---|---|---|---|
| S1-1 type annotations rejected | fixed; var statements fail loudly | `bzl_compile.rs` `should_use_type_syntax`, starlark-rust patch 0020 | `review_2026_11_18_test::bzl_type_annotations_*`, `patch_0020_bazel_type_syntax` |
| S1-2 prelude drops load-bound and `_` names | fixed | `package_function.rs`, patch 0021 | 13 `test_prelude_*` of PackageFunctionTest ported (10 run, 3 ignored for starlark-rust error text) |
| S1-3 package-loading flags ignored | fixed in blaze_sky_loading and `blaze-query-dev`; the main binary needs two calls from the lead | `skyframe_executor.rs`, `pkgcache.rs` | `deleted_packages_are_not_packages`, `default_visibility_option_*`, `package_path_option_*`; oracle run below |
| S1-4 `checkAspectAllowedValues` | fixed (literal port) | `rule_class.rs`, `aspects_list.rs` | `aspect_allowed_values_*`, 20 StarlarkDefinedAspectsTest ports |
| S1-5 `select()` positional `no_match_error` | fixed | `build/select.rs` | `select_accepts_a_positional_no_match_error`, SelectTest ports |
| S2-1 allowlist repo and package match | fixed | `rule_class_functions.rs` | `allowlist_entry_*` (blaze_packages), `initializer_*_private_attribute_*` |
| S2-2 process-global module context registry | fixed in the owned crates; three call sites remain in the lead's crates | `bzl_context.rs`, `defining_module.rs`, patch 0022 | `patch_0022_def_module_frozen_extra_value` |
| S2-3 `Label()` falls back to the BUILD package | fixed (no fallback) | `blaze_starlark/src/label.rs` | prelude and initializer ports |
| S2-4 non-label `attr.*` default type | fixed | `attr_module.rs` | `test_function_as_attr_default`, `attr_default_type_errors_*` |
| S2-5 initializers see the package builder | fixed | `rule_class_functions.rs`, `thread.rs` | `initializer_fails_creating_another_rule`, `initializer_fails_with_existing_rules`, `initializer_native_module` |
| S2-6 immutable values are mutable | fixed for lists and dicts; `string_set` deferred | `type_.rs`, `attribute.rs`, `starlark_native_module.rs` | `initializer_cannot_mutate_lifted_*`, `existing_rule*_immutable*` |
| S2-7 `--max_computation_steps` | fails loudly when set | `package_factory.rs` | none (a panic path) |
| S2-8 `@Param` Sequence checks | fixed | `starlark_native_module.rs`, `build_globals.rs` | `test_glob_enforces_list_*`, `build_natives_*`, `glob_*` |
| S2-9 guarded-symbol errors | fixed for .bzl; BUILD path fails loudly (unreachable with Bazel's provider) | `builtins/env.rs`, `bzl_load.rs`, `starlark_util.rs` | `review_s2_builtins_test` (6) |
| S2-10 `repository_name` flag | fixed; the review's reading is disputed in part | `starlark_native_module.rs` | `repository_name_*` |
| S2-11 Java hash order | fixed; identity-hashed value types fail loudly | `aggregating_attribute_mapper.rs`, `computed_default.rs`, `java_concurrent_hash_map.rs` | 20 unit tests, 3 loading tests |
| S2-12 stubs answer every name | fixed, also for `cc_internal` and `java_common` | `builtins/stubs.rs`, `cc_internal.rs`, `java_common.rs` | `stubs.rs::tests`, `android_common_*` |
| S2-13 struct with a mutable field is hashable | fixed | `blaze_starlark/src/structs.rs` | `structs_in_dicts` (port of testStructsInDicts) |
| S3-7 empty ignored stubs | 294 Java tests ported for real; the stubs of the files touched got current reasons. Corrected in round 2: three files were not touched (`bzl_load_function_test.rs`, `package_group_test.rs`, `environment_group_test.rs`) and still had stale empty stubs; see "Round 2 response" | below | below |

## S1

**S1-1.** Fixed. `BzlCompileFunction.shouldUseTypeSyntax` (skyframe/BzlCompileFunction.java:249)
is ported into BZL_COMPILE, with the flag, the allowlist (`label.getCanonicalForm().startsWith`)
and the prelude and .scl exclusions. Java's mode at 9.3.2 is "parse and ignore":
`allowTypeSyntax(true)`, `resolveTypeSyntax(false)`, `tolerateInvalidTypeExpressions(true)`
(:212-214), so annotations are arbitrary expressions that are neither resolved nor evaluated.
starlark-rust's `DialectTypes::ParseOnly` did not match that (it checked the shape of type
expressions, resolved their names and evaluated them at compile time), so patch 0020 changes
`ParseOnly` to Java's behaviour and adds `cast(T, e)`, the `isinstance` resolver error, `type X =
T` and `def f[T](...)`. `--experimental_starlark_type_checking` is `unported!`. Not ported: a var
statement (`x: T` without a value); the parser returns a marked error that BZL_COMPILE turns into
`unported!`, not a user syntax error. Known small difference: `...` is accepted outside
annotations.

**S1-2.** Fixed. The prelude's bindings are now every binding of the module (patch 0021,
`FrozenModule::names_any_visibility`), as `prelude.getGlobals()` (skyframe/PackageFunction.java:1068).

**S1-3.** Fixed in blaze_sky_loading. New: `PathPackageLocator::create` (pkgcache/PathPackageLocator.java:159),
`LoadingFunctionsConfig::sync_package_loading(&PackageOptions)` (package path, deleted packages,
`show_loading_progress`) and `inject_loading_precomputed_with_package_options` (default
visibility, config_setting visibility policy, lazy macro packages), following
`SkyframeExecutor.preparePackageLoading` and `syncPackageLoading` (skyframe/SkyframeExecutor.java:1693, 3035).
`blaze-query-dev` uses them. The existing entry points keep their signatures and the option
defaults. `LazyMacroExpansionPackages.ALL` is `unported!`.

Not wired: the `blaze` binary. `blaze_sky_top::SkyframeExecutor::create` does not receive
`PackageOptions`, and none of the owned crates sees the parsed options there, so a loud failure
from inside them is not possible. Until the lead adds the two calls (see Requests), the main
binary still ignores these flags silently.

Oracle check, scratch workspace with `a/BUILD` (`filegroup(name = "x", srcs = glob(["**"]))`)
and `a/b/BUILD`, oracle against `blaze-query-dev`, `--output=label_kind`:

| query | oracle and Blaze |
|---|---|
| `//...` | `//a:x`, `//a/b:y`, exit 0 |
| `--deleted_packages=a/b //...` | `//a:x` only, exit 0 |
| `--deleted_packages=a/b 'deps(//a:x)'` | `//a:BUILD`, `//a:b/BUILD`, `//a:b/f.txt`, `//a:top.txt`, `//a:x`, exit 0 |
| `--deleted_packages=a/b //a/b:all` | exit 7, "no such package 'a/b': Package is considered deleted due to --deleted_packages" |

Stdout, exit code and error text are identical in all four.

**S1-4.** Fixed. `checkAspectAllowedValues` (packages/RuleClass.java:2158) is ported literally on top
of `getDefaultParametersExtractor` and `addAttrValue` (packages/StarlarkDefinedAspect.java:299, 265).
Both of the review's cases are tests. Native aspects have no attribute model at loading, as
before.

**S1-5.** Fixed: `no_match_error` is positional-or-named.

## S2

**S2-1.** Fixed. `AllowlistEntry.reposMatch` and `allows` (packages/BuiltinRestriction.java:124-150)
are ported with the repo mapping and `PathFragment.startsWith`. The copy in `builtins/stubs.rs`
already matched Java.

**S2-2.** Fixed in the owned crates. The module context is read from the function's own module:
patch 0022 keeps the module's extra value in the frozen module data, and
`BazelModuleContext::of_function`, `of_innermost_enclosing_starlark_function` and
`of_innermost_bzl` read it (a function whose module is still executing belongs to the evaluator's
module). Two modules executed from the same file each keep their own context. The table keyed by
file name is still filled, because `blaze_analysis` and `blaze_rules` read it (`of_filename`,
`all_package_contexts`); nothing in the owned crates does. It stays ambiguous for those callers
until they move (see Requests).

**S2-3.** Fixed. `Label()` resolves against the calling function's module, whichever thread runs
it, so prelude-loaded and `@_builtins` modules need no registration. With no module context it
fails with Java's text ("Label() can only be used during .bzl initialization (top-level
evaluation)"). An explicit `ThreadContext::with_file_package_context` registration is still
honoured for modules executed without an extra value (blaze_starlark's own tests).

**S2-4.** Fixed for every `attr.*` function, with the interpreter's text
(`net/starlark/java/eval/BuiltinFunction.java:498`). Beyond the finding, the label-typed
functions are checked too, so `attr.label_list(default = configuration_field(..))` is now
rejected, as in Java.

**S2-5.** Fixed. While initializers run the payload is off the thread and the package's
`LabelConverter` is a thread-local (analysis/starlark/StarlarkRuleClassFunctions.java:1789-1792,
1880); a drop guard restores both on every exit. `select()` and `package_relative_label` read the
thread-local first.

**S2-6.** Fixed for lists and dicts: lifted attribute values and the `existing_rule(s)` views
are immutable (packages/Type.java:598, 770; packages/StarlarkNativeModule.java:184, 201, 216).
Deferred: `string_set` values (Type.java:846); the vendored crate has no immutable set, and
failing there would break every `attr.string_set` lift. Also open: `existing_rule()` returns an
immutable dict where Java returns a view, so the error text of an assignment differs (two ported
tests are ignored for that).

**S2-7.** Deferred to a loud failure. starlark-rust does not count Java's steps, so
`--max_computation_steps > 0` is `unported!` before the BUILD file runs.

**S2-8.** Fixed for `glob`, `subpackages`, `exports_files`, `package_group`,
`environment_group` and `licenses`. A range passes the Sequence check and fails on its elements
with `Types.STRING_LIST.convert`'s text, not `Sequence.cast`'s as the review expected
(StarlarkNativeModule.java:102).

**S2-9.** Fixed for .bzl files: a use of a guarded symbol reports the guard's message alone at
the identifier and the load fails as a compile error (`net/starlark/java/syntax/Resolver.java`
`use`). For BUILD files the path is `unported!`; Bazel 9.3.2 registers no guarded BUILD
toplevel, so it is unreachable with Bazel's provider. Remaining differences: starlark-rust stops
at the first unresolved name; other undefined names still fail as "initialization of module".

**S2-10.** Fixed, with a correction to the finding. `repository_name` is always bound: the
native module's methods are selected under default semantics (StarlarkNativeModule.java:88), and
the flag is checked when the function is called (`net/starlark/java/eval/BuiltinFunction.java:262`).
The call now fails with Java's "function repository_name() is experimental and thus unavailable
with the current flags...". No composing-code change was needed.

**S2-11.** Fixed. Duplicate labels come out in `HashSet<Label>` order, and the computed-default
lookup table in `HashMap` order with Guava's and the JDK's capacities; the order does reach
output (Attribute.java:1563 and the "Available keys" message). Expected orders in the tests were
produced by the pinned server jar on a JDK. Loud failures: a treeified bin, and a table of two or
more entries whose key contains a License, TriState or default-computing object (identity hash
in Java).

**S2-12.** Fixed. Each stub answers exactly the `@StarlarkMethod` names of its Java class;
`hasattr`, `getattr` with a default and `dir` behave as in Java; calling a stub method, or
reading an unported struct field, is `unported!`. Applied to `cc_common`, `apple_common`,
`android_common`, `py_builtins`, `cc_internal` and `java_common`.

**S2-13.** Fixed. A struct hashes only if it is immutable (`net/starlark/java/eval/StarlarkValue.java:104`).
A struct with a frozen list still hashes, as Java's comment says. The blaze_starlark test that
asserted the opposite was not from Java and is replaced by a port of `testStructsInDicts`.

## Tests (S3 item 7)

The harness is `package_loading_test_case.rs`: a scratch workspace on a temp directory and the
real loading SkyFunctions with Bazel's rule class provider. Tests that load packages live in
blaze_sky_loading because blaze_packages cannot depend on it.

| Java class | ported |
|---|---|
| PackageFunctionTest | 43 new (all 13 prelude tests) |
| PackageFactoryTest | 87 |
| StarlarkRuleClassFunctionsTest | 85 |
| StarlarkDefinedAspectsTest (aspect parameters) | 20 |
| NativeSubpackagesTest, OutputFileTest, RuleTest | 14, 10, 9 |
| ExportsFilesTest, NativeGlobTest, InputFileTest | 4 each |
| SelectTest, NativeExistingRulesTest | 3 each |
| rule-instantiation errors from four analysis test classes | 8 |

Stubs left: `starlark_defs/tests_ignored.rs` 170 (was 261), `build/loading_tests.rs` 36 (was
170), each with a reason that names what the Java test needs (analysis, symbolic macros, Java
object inspection). The five rule-instantiation strings the review listed each have a test now.

Ported tests found and fixed these further bugs: `setContainsErrors` failed for errors raised by
`finalBuilderValidationHook` (five `environment_group` tests panicked;
packages/TargetDefinitionContext.java:491 checks `pkg.targets == null`); the rule function's
traceback name was `<rule my_rule>` instead of `my_rule`; `attr.label_list(providers=)` checked
sublists before the outer cast (StarlarkAttrModule.java:592); `aspect(propagation_predicate=)` and
`rule(attrs=)` lacked their parameter type checks. Patch 0023 gives Java's text for mutating a
frozen list or dict.

27 ported tests are ignored with their real bodies:

- 20 assert interpreter text that starlark-rust words differently (argument binding, undefined
  names, division by zero, unsupported operands); decisions.md accepts that text and patch 0010
  is queued. Two more are the repr of builtins (patch 0009) and a duplicate keyword argument.
- `test_package_records_loaded_modules`: Declarations store precomputed transitive loads; Java's
  default stores direct loads, so a custom visitor sees a twice-loaded module twice.
- two `existing_rule(s)` assignment tests (the view type, above).
- `initializer_allowlist`: the allowlist is checked at analysis.
- `test_rule_class_too_long_attribute_name`: export errors are not reported as located events
  (review S4).

## Verification

- Tests, six crates (`blaze_packages`, `blaze_sky_loading`, `blaze_starlark`, `blaze_query`,
  `blaze_query_engine`, `starlark_conformance`), full run: 1036 passed, 0 failed, 641 ignored.
  blaze_packages lib went from 338 passed / 489 ignored to 367 / 264; blaze_sky_loading from 124
  / 147 to 429 / 159.
- `scripts/w3/query-compare`: brotli 162/162, re2 1091/1091, workerd 46371/46371 on both
  surfaces, exit codes equal.
- `cargo check --workspace --all-targets` passes.
- `rustfmt --edition 2024` clean on every changed file. Clippy with `-D warnings` is clean in the
  owned files; three findings remain in the skyframe agent's files (Requests).

## Requests

Integration lead:

1. `blaze_sky_top/src/skyframe_executor.rs`: with the command's `PackageOptions`, call
   `loading_config.sync_package_loading(&package_options)` before `register_loading_functions`,
   and replace `inject_loading_precomputed(..)` by
   `inject_loading_precomputed_with_package_options(.., &package_options)`. Until then the main
   binary ignores `--deleted_packages`, `--default_visibility`, `--package_path` and the
   config_setting visibility flags.
2. Replace `BazelModuleContext::of_filename(&module_file_of_...)` by
   `BazelModuleContext::of_innermost_enclosing_starlark_function(eval, depth)` or
   `of_function(value, eval)` in `blaze_analysis/src/rule/builtin_restriction.rs:139`,
   `blaze_analysis/src/starlark/starlark_action_factory.rs:1749` and
   `blaze_rules/src/cpp/cc_starlark_internal.rs:154`; drop the `all_package_contexts()` loops in
   `starlark_toolchain_context.rs:46` and `starlark_rule_configured_target_util.rs:168`
   (`Label()` no longer needs them).
3. Behaviour changes to expect: `Label()` no longer falls back to the thread's file label; a
   struct with a mutable field no longer hashes; hashing fails where Java would hash such a
   struct through `hashCode` without `checkHashable`.

Skyframe-fix agent: clippy findings at `file_state.rs:151`, `target_pattern_function.rs:440` and
`:925`. I ran `clippy --fix` over the owned crates and restored `file_state.rs` and
`target_pattern_function.rs` from a copy taken immediately before that run; an edit made to
either file during those two minutes would have been overwritten and should be checked.

## Open

- Var statements, `string_set` immutability, the BUILD-file guard path, and the three flags'
  wiring in the main binary, as stated above.
- `Package.Declarations` should store direct loads (one ignored test).
- starlark-rust reports only the first unresolved name; Java reports all.
- `RuleClassTest`, `RuleFactoryTest`, `PackageTest` and `RawAttributeMapperTest` stubs are
  blocked only on the writing (a `Package.Builder` fixture exists in
  `blaze_packages/tests/package_builder_test.rs`).

# Round 2 response

Date 2026-11-18. Answers round 2 of `2026-11-18-loading.md` (verdict STOP on S1-6).

Tree: HEAD `03409d9` plus uncommitted changes in the owned crates, `third_party/starlark-rust`,
`scripts/w3/deleted-packages-probe`, `scripts/w3/deleted-packages/` and this file. Nothing is
committed. Other agents have uncommitted work in the same tree.

| item | state | where | test |
|---|---|---|---|
| S1-6 `proto` not bound | fixed (full port) | `blaze_packages/src/proto.rs`, `blaze_sky_loading/src/environment.rs` | 5 Java proto tests, 6 oracle-backed tests, 2 unit tests |
| S2-6 `string_set` | immutable set added (patch 0025) and used; the finding's example is disputed | `type_.rs`, `set/value.rs`, `set/refs.rs` | `patch_0025_immutable_set`, `copy_and_lift_starlark_set_is_immutable` |
| stale stubs in three files | 68 ported (67 run); 4 stay empty stubs with accurate reasons | below | below |
| `--deleted_packages` evidence | reproducible | `scripts/w3/deleted-packages-probe`, `scripts/w3/deleted-packages/ORACLE.txt` | `deleted-packages-probe compare`: MATCH, 5 commands |

**S1-6.** `proto.encode_text` is a literal port of packages/Proto.java:111-232 (`encodeText`,
`TextEncoder.message`, `field`, `fieldElement`, `emitLine`) and is bound with `depset` and `json`
in `bazel_starlark_globals_fns`, which `blaze_rules` builds on, so the `blaze` binary gets it too.
Floats go through a port of `StarlarkFloat.toString` (net/starlark/java/eval/StarlarkFloat.java:51,
120). The parameter error says "want 'structure or NativeInfo'", which is what Java and the oracle
print (`Starlark.classType(Structure.class)`), not "struct". At the pin `NativeInfo` is not a
`Structure`: it is accepted as the argument and rejected when nested, and the port follows that.
Two `unported!` spots: nesting deeper than 1000 (Java overflows its stack) and a `range` whose
iteration fails (unreachable). The five Java tests (`testSimpleTextMessages`, `testNoneStructValue`,
`testProtoFieldsOrder`, `testTextMessageEscapes`, `testTextMessageInvalidStructure`) are ported;
the seven `json.encode` tests that shared their stubs are ported too. Small differences left:
`type(proto)` and `repr(proto)` are starlark-rust's namespace values, as for `json`.

**S2-6 `string_set`.** Patch 0025 adds immutable sets to the vendored crate, mirroring 0016 and
0018, and gives Java's text for mutating a frozen set; `Type::StringSet` lifting uses it
(packages/Type.java:846) and `is_immutable` answers for sets. The review's example cannot be run
on either side: Bazel 9.3.2 has no `attr.string_set` (lib/starlarkbuildapi/StarlarkAttrModuleApi.java
has no such method; `AttributeInfoExtractor.java:127` says "STRING_SET is not exposed to Starlark
attr API"). The only `STRING_SET` attribute is `build_setting_default` of `config.string_set`
(packages/RuleClass.java:907), which is not Starlark-defined, and every caller of
`copyAndLiftStarlarkValue` requires a public Starlark-defined attribute
(analysis/starlark/StarlarkRuleClassFunctions.java:1810, 1858-1868; packages/MacroClass.java:274,
295). So the lifted set never reaches an initializer and the fix has a unit test, not a loading
test. Not done: `x |= y` on an immutable set rebinds in starlark-rust where Java mutates and
fails (queued patch 0007). The patch number is 0025 because another agent took 0024.

**Stale stubs.** The round-1 response was wrong to say every remaining stub had a current reason;
these three files were not touched then. Now:

| Java class | ported now | ignored | file |
|---|---|---|---|
| BzlLoadFunctionTest | 40 | 6 | `bzl_load_function_test.rs` (61 tests in all) |
| PackageGroupTest | 23 | 1 | new `package_group_test.rs` in blaze_sky_loading |
| EnvironmentGroupTest | 5 | 1 | new `environment_group_test.rs` in blaze_sky_loading |

The ignored ones: two need a file system that fails one stat or blocks (empty stubs, Java body
summarised); two `reduceForSerialization` tests; one holds a raw 0xFF byte (non-UTF-8 Starlark
files are `unported!`, review S3 item 2); two hold interpreter wording only (split out as
`_interpreter_text`). None of the old "needs agent B's visibility()" stubs needed anything
missing. Also done from the round's smaller items: the two `select_test` tests are split so the
ten "Cannot combine incompatible types" assertions run; the `existing_rule(s)` views fail an
assignment with Java's text (net/starlark/java/eval/EvalUtils.java:522) and their two tests run.

One more fix found by the new tests: an undefined name in a .bzl or .scl file now fails as
"compilation of module ... failed" with the error at the identifier, as Java's compile step does,
instead of "initialization of module ... failed" (`starlark_util::undefined_name_syntax_error`).
The message itself stays starlark-rust's.

**`--deleted_packages` evidence.** `scripts/w3/deleted-packages-probe record` writes the oracle's
transcript of five queries on `scripts/w3/deleted-packages/ws` to `ORACLE.txt` (a fresh server
per command); `compare` runs `blaze-query-dev` and diffs. Result: MATCH, 5 commands. It uses
`blaze-query-dev` because the `blaze` binary still lacks the two `blaze_sky_top` calls.

## Verification (round 2)

- Tests, the same six crates, full run: 1131 passed, 0 failed, 562 ignored (round 1: 1036 / 0 /
  641).
- `scripts/w3/query-compare`: brotli 162/162, re2 1091/1091, workerd 46371/46371.
- `cargo check --workspace --all-targets --keep-going` fails in `blaze_sky_exec` only
  (`action_execution_function.rs:223, 575, 785`, against another agent's uncommitted
  `skyframe_action_executor.rs`). Crates that depend on it (`blaze_sky_top`, `blaze_runtime`,
  `blaze`) could not be checked in this tree. Round 2 changed no existing public signature.
- rustfmt clean; clippy clean in the owned files (the same three findings in
  `file_state.rs` and `target_pattern_function.rs`).
- `patches/blaze.diff` regenerated (covers 0020 to 0025).

## Still open after round 2

- For the integration lead: the two `blaze_sky_top` calls (S1-3), and the file-name-table
  readers (S2-2); round 2 lists two more than the round-1 response did
  (`blaze_repo_rules/src/starlark/repository_module.rs:531, 603`).
- Round-2 S3/S4 items not addressed: `StarlarkTypesTest` is not ported and the `.scl` type
  annotation explanation has no test; 17 `NativeExistingRulesTest` stubs remain (they need a
  test-only `test.save` toplevel or `@rules_cc`); the three S4 cases Blaze accepts under type
  syntax; `print(Label(..))` in an initializer; `ThreadContext::package_context_for_file` is dead
  code.
- A shared `StarlarkFloat` formatter: `blaze_bzlmod_model/src/facts.rs:195` has a second port
  without Java's two-digit minimum (prints `5e-324` where Java prints `4.9e-324`).
