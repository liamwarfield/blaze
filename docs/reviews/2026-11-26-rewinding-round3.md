# Review rewinding, round 3 (2026-11-26)

Commit under review: 8ecc520 (checkpoint 46), read with `git show 8ecc520:<path>` and
`git diff b216603 8ecc520`, not from the working tree. Reviewer: read-only, no Cargo. No Bazel or
Blaze process was started: the suite unit `root-suite-5f68157` (now running the 8ecc520 suites)
was active for the whole review. Everything here is from reading the Rust against the pinned Java;
no test was run by me.

Verdict: **STOP** (blocking: IS-2, new, a one-word fix). AT-1 is fixed.

## Scope

Read: every `dict_cast` / `sequence_cast` / `sequence_noneable_cast` call site of
`blaze_packages/src/starlark_defs/{rule_class_functions,config,attr_module,aspect,util,provider}.rs`
at 8ecc520 and the nine changed lines of the diff; the diff of
`crates/blaze/tests/rewind_lost_inputs_test.rs`, `blaze_sky_exec/src/{action_execution_function,
action_execution_value,keys,skyframe_action_executor,discovery_env}.rs`,
`blaze_rules/src/cpp/{cc_compilation_context,cpp_compile_action}.rs`,
`blaze_rules/src/includescanning/{cpp_include_scanning_context_impl,legacy_include_scanner}.rs`,
`blaze_analysis/src/test/starlark_testing_module.rs`. Java: `Starlark.java:375-395`,
`Sequence.java:132-145`, `ParamDescriptor.getTypeErrorMessage`, `BuiltinFunction.java:402-409`,
`StarlarkRuleClassFunctions.java:1837`, `StarlarkAttrModule.java:456, 465`,
`ConfigGlobalLibrary.java:128`, `StarlarkTestingModule.java:60-86`, `TestingModuleApi.java`
(`analysis_test` parameters), `CcCompilationContext.java:240-262, 355-395`,
`CppCompileAction.java:600-660, 722-762, 807, 2068-2100`, `DiscoveredModulesPruner.java:50-85`,
`CppIncludeScanningContextImpl.java:92-172`, `LegacyIncludeScanner.java:628-636`,
`collect/compacthashset/CompactHashSet.java:372-440`.

Not read: `output_checker.rs` (new file, not rewinding), `rewinding_test.rs` and
`artifact_function_test.rs` (tests), `recursive_filesystem_traversal_function.rs`, the
`rewinding/` directory and `completion_function.rs` (no change in the diff), `HeaderInfo`'s
`get_transitive_collection`/`get_separate_module` ports, `NestedSet::shallow_equals` beyond its
body, `Environment::get_values_and_exceptions`.

## AT-1: FIXED

Java `Starlark.classType(Object.class)` is `"unknown"` (`Starlark.java:386-390`). At 8ecc520 every
port of a cast to `Object.class` passes `"unknown"`:

| Rust | Java |
|---|---|
| `rule_class_functions.rs:1464-1471` (`attr_values`) | `StarlarkTestingModule.java:83` `Dict.cast(.., String.class, Object.class, "attr_values")` |
| `rule_class_functions.rs:1172-1179` (initializer result) | `StarlarkRuleClassFunctions.java:1837` |
| `config.rs:346-353` (`changed_settings dict`) | `ConfigGlobalLibrary.java:128` |
| `provider.rs:469-476` (init result; was right) | `StarlarkProvider.java:302` |
| `rule_class_functions.rs:784` (`toolchains`), `attr_module.rs:579` (`values`), `:591` (`providers`), `aspect.rs:1160, 1241, 1250` | `Sequence.cast`/`noneableCast(.., Object.class, ..)` (`StarlarkAttrModule.java:456, 465`; these can never print the name) |

No `"Object"` or `"any"` literal is left in `starlark_defs`, `blaze_analysis/src/{test,starlark}`
or `blaze_cc_features` (grep). No site was wrongly changed: the casts with a real element class
keep its `classType` name (`"string"`, `"Attribute"`, `"exec_group"`, `"Subrule"`, `"Aspect"`,
`"Provider"`). (The code change is in a2b67af, checkpoint 44, not in c434f6c, which holds only
documents; that does not matter for the result.)

## Round-2 S2 items

| item | state | where (8ecc520) |
|---|---|---|
| RW-6 the test cannot tell a rewind from a build retry | FIXED | `crates/blaze/tests/rewind_lost_inputs_test.rs`: `--experimental_remote_cache_eviction_retries=0` on the rewind build and `!err.contains("retrying the build")` |
| IS-1 interrupt message dropped | FIXED | `legacy_include_scanner.rs:580-586`: `"Include scanning interrupted while {operation} {source}"` at both call sites (`processing`, `visiting`); still not printed, as Java |
| AT-2 export errors cut to the first | FIXED | `rule_class_functions.rs:1570-1575`: every error after a newline, `"Errors in exporting {name}: {errors}"` |
| S3 no shared-action rewind test | answered | new `shared_action_copies_rewound_once` (ignored by default, needs the remote worker; expectations recorded from the oracle per its header) |
| S4 order of BUILD-thread check and parameter types | FIXED | `starlark_testing_module.rs:109-139`: the `@Param` type errors first, with `getTypeErrorMessage`'s names (`string`, `function`, `dict`, `sequence`) |

The response file `2026-11-26-rewinding-response.md` has no round-2 section; the states above are
from the code.

## New findings

### S1

- **IS-2 (blocking). `computeUsedModules`: the own module is removed with the wrong order
  semantics.** Rust `crates/blaze_rules/src/cpp/cc_compilation_context.rs:335`
  `modules.shift_remove(own)` keeps the order of the remaining modules. Java
  `CcCompilationContext.java:358, 393`: `modules` is Bazel's `CompactHashSet`, whose `remove`
  moves the **last** entry into the removed slot (`CompactHashSet.java:393, 407-415` `moveEntry`),
  so the iteration order changes. Input: a compile with header modules and include scanning whose
  scan finds headers of the modules `[A, own, B, C]` (in transitive header-info order). Java's
  used modules are `[A, C, B]`; Blaze's are `[A, B, C]`. That order is the order of `topLevel`
  (`LinkedHashSet` of `usedModules`), of the `topLevelModules` nested set (the `-fmodule-file=`
  arguments of the compile) and of the discovered modules added to the inputs: a different
  command line and action key, silently. At b216603 this path stopped with `unported!`
  (`findUsedModules`); it now returns a value. Fix: `swap_remove` (IndexSet's `swap_remove` is
  exactly `moveEntry`). `compute_top_level_modules` (`cpp_compile_action.rs:1177`) is right to use
  `shift_remove`: Java's set there is a `LinkedHashSet`. Not probed (no header-module toolchain in
  the corpus; suite running).

### S2

- **IS-3. A failed Skyframe read in discovery is folded into "values missing".**
  `blaze_sky_exec/src/discovery_env.rs` `get_tree_artifact_values` and `get_discovered_modules`
  use `get_values_and_exceptions(..).ok()?`: whatever error that call returns (an interrupt, by its
  Java signature) becomes `None`, a restart, where Java throws `InterruptedException` out of
  `discoverInputs`. Masked: the evaluator sees the interrupt elsewhere. Fix: carry the error.

### S3

- Header modules with scanning and tree artifacts among sources/headers are now ported (were
  `unported!`); no test or oracle case for them was shown. `DiscoveredModulesPruner` is fixed to
  `DEFAULT` (declared). `/showIncludes`, grep-includes, include hints, swig stay marked.
- `RewindingTest` cases and the two end-to-end tests stay `#[ignore]` by default.

### S4

- `starlark_testing_module.rs:120-123`: `is_sequence` accepts list and tuple; a Java `Sequence`
  is also a `range`.

## Skim of the rest (faithful by reading)

- `expand_tree_artifacts` and `handle_tree_artifacts` follow Java's order (children appended in
  place of each tree; one all-or-nothing read). `find_minimal_set_of_required_modules`,
  `compute_top_level_modules`, the tail of `discover_inputs` (`usedModules` kept across a restart,
  `discoveredModules` only for a non-C++20 `.pcm` output) and
  `ActionExecutionValue::with_discovered_modules` (Java `create`'s three argument checks,
  `ModuleDiscovering`) match the Java lines above. `values_equal` now also compares the discovered
  modules with `shallowEquals`.
- `discovery_env.rs` lends the node's environment through a scoped thread-local slot (declared
  PORT); discovery runs synchronously on the function's thread. Rewinding paths
  (`skyframe_action_executor.rs` lost-input handling) are unchanged apart from taking the
  environment instead of its listener.

## Verdict

**STOP**, bound to 8ecc520. Blocking: IS-2 (`cc_compilation_context.rs:335`: `shift_remove` where
Java's `CompactHashSet.remove` moves the last entry into the hole, so the used-module order, and
with it the compile's module arguments, differs). AT-1 is fixed; nothing else blocks.
