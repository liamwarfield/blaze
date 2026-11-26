# Review, round 2: java-config

Date 2026-11-26. Reviewer: Fable 5.2, read-only. Commit under review: **5f68157**.
Re-check of `2026-11-25-java-config.md` (STOP at e8b79c0, blocking S1-1) against
`2026-11-25-java-config-response.md`.

Verdict: **PASS**, bound to 5f68157. S1-1 is fixed. One new S2 (R1): under
`--experimental_output_paths=strip`, five execution sites can run an action unmapped without the
loud stop the response describes.

No Blaze or Bazel process was started (`root-suite-5f68157` was active throughout), no Cargo.
Everything is from reading the Rust against the pinned Java. The response's probes (`aq-strip`,
`cq-tr`, `b-stamp` and the table of its section 4) were not re-run.

## Scope

Read: the diff e8b79c0..5f68157 of `java_header_compile_action.rs`, `path_mappers.rs`,
`stripping_path_mapper.rs` (all of it), `spawn_action.rs` (blaze_actions),
`parameter_file_write_action.rs`, `custom_command_line.rs`, `starlark_file.rs`, the path-mapper
lines of `starlark_custom_command_line.rs`, `build_info_file_write_action.rs` (blaze_actions and
blaze_exec), `function_transition_util.rs`, `starlark_transition.rs`, the mapper sites of
`cpp_compile_action.rs` and `java_compile_action.rs`, `blaze_exec/src/analysis/actions/
spawn_action.rs` `get_spawn_with_env`, the harness functions of S2-2 and S2-3; every call of
`path_mappers::create`, `create_for_execution` and `SpawnPathMapper::NOOP` in `crates/`. Java:
`StrippingPathMapper.java`, the seven `PathMappers.create` call sites, `SpawnAction.computeKey`,
`BuildInfoFileWriteAction.execute`, `WorkspaceStatusAction.parseValues`, `Dict.cast`,
`FunctionTransitionUtil.java:668-684`, `BuildViewTestCase.findArtifactNamed/artifactNamed`.

Not read: the 356 re-pointed `// Java:` markers (S4), the other changed files of
`blaze_rules/src/java` (marker changes by the diff stat), the moved Java-rule tests in
`blaze_sky_top/src/tests`, the rest of the harness diff (`get_bin_artifact`,
`artifacts_to_strings`, the callers moved to `UpdateError`), `workspace_status_action.rs`
`parse_values_in_insertion_order`, `java_hash_map_order`, `digest_utils.rs`,
`file_artifact_value.rs`, `StringStripper` beyond its loop, `mapRoot` callers.

## Round-1 findings

| finding | state at 5f68157 |
|---|---|
| S1-1 (blocking) | **fixed**: `java_header_compile_action.rs:186-191` passes the empty set to `add_to_fingerprint`, as `SpawnAction.computeKey` (SpawnAction.java:410-416). The second fix is right too: `get_arguments` (`:273-286`) calls `path_mappers::create(self, ...)`, so the mapper sees this class's execution info and additional artifacts (inherited `SpawnAction.getArguments`, SpawnAction.java:207). Key equality under `strip` rests on the response's `aq-strip` probe. |
| S2-1 | fixed: `map_option_value` stores `String=Label` entries for `bytecode_optimizers`, the empty dict for `test_timeout`, and is `unported!` for anything else; the changed-test compares the map without order. |
| S2-2 | fixed: `find_artifact_named` / `artifact_named` (`build_view_test_case.rs:241-263`) compare `pretty_print()` for equality and panic with Java's text. |
| S2-3 | fixed: `UpdateError::TargetParsing` (`:105-121`, `:1923`). The two tests that changed result catch both exceptions in Java as well. |
| S4 markers | not re-checked beyond the header action's (correct at the pin where I looked). |

## Findings

### S1

None.

### S2

**R1. Under `strip`, an action whose inputs collide runs unmapped without the loud stop.**

- Rust: `path_mappers::create_for_execution` (`crates/blaze_actions/src/analysis/actions/
  path_mappers.rs:123`) stops only when the mapper it gets is not the no-op. Five of the six
  execution sites pass `None` as the input metadata provider:
  `parameter_file_write_action.rs:333`, `cpp_compile_action.rs:1734` and `:2064`,
  `java_compile_action.rs:493`, `java_header_compile_action.rs:320`. With `None`,
  `have_same_digest` (`stripping_path_mapper.rs:360-366`) answers false for two inputs that map
  to the same stripped path, `is_path_strippable` is false, `try_create` gives nothing, the
  mapper is the no-op, and the action runs with unmapped paths. No `unported!` is reached.
- Java: all of these sites pass `actionExecutionContext.getInputMetadataProvider()`
  (ParameterFileWriteAction.java:207, CppCompileAction.java:1487 and :1702,
  JavaCompileAction.java:313 and :367, SpawnAction.java:368). With a provider, two colliding
  inputs with equal digests are allowed (StrippingPathMapper.java:340-376) and the action runs
  path-mapped.
- Input: `build --experimental_output_paths=strip` of a `java_library` whose compile action has
  the same jar from two configurations with the same content (a library that is both a
  dependency and on a plugin's or tool's classpath). Bazel maps the spawn; Blaze runs it
  unmapped and reports success. When the digests differ both run unmapped, which is right.
- Masked: the outputs of a mapped and an unmapped Javac or compile spawn are meant to be the
  same; what differs is the spawn (arguments, staged paths, remote cache key) and the content
  of a parameter file. So S2, not S1. But the response states that execution under `strip`
  stops loudly, and for this case it does not.
- The sixth site is right: `blaze_exec/src/analysis/actions/spawn_action.rs:91` passes the
  provider, so a collision reaches `unported!("StrippingPathMapper.haveSameDigest")`.
- Fix expected: pass the provider at the five sites (the collision then reaches the existing
  `unported!`), or make `create_for_execution` stop whenever the effective mode is STRIP and
  `try_create` declined because of a collision.

### S3

- Running a path-mapped spawn (`Spawn.getPathMapper`, staging, `.jdeps` rewriting):
  `unported!` in `create_for_execution`, apart from R1.
- `StrippingPathMapper.mapCustomStarlarkArgs` for the Android mnemonics, `haveSameDigest` with
  a provider, Map-typed transition values of other entry classes: `unported!`.
- 28 of 38 `JavaStarlarkApiTest` methods and `testExtraArgsPropagated` stay `#[ignore]` with
  reasons (response section 5).

### S4

None new.

## Checked and found equal

- No execution path maps silently. `path_mappers::create` (the mapping one) is called only from
  `SpawnAction::get_arguments` and the header action's `get_arguments`; their consumers are
  aquery, the BEP action events and `getStarlarkArgv`, which use the mapped form in Java too
  (null provider there as well, so display agrees on collisions). Every spawn is still built
  with `SpawnPathMapper::NOOP`, and each of those sites sits behind `create_for_execution`.
- `StrippingPathMapper`: `tryCreate` input order, `map`, `getMappedExecPathString`,
  `computeExecPathLengthDiff`, `getMapFn` (the four Java mnemonics, `--javacopts` and
  `--resources`), `mapHeuristically`, `mapRoot` with the default body, `strip`,
  `isPathStrippable`; `structuredArgStripper` strips a `String` and expands anything else as
  `expandToCommandLine` (`custom_command_line.rs`). The `map_each` mapper is a scoped
  thread-local read on the same thread that runs the callback (`with_evaluator` does not
  switch threads).
- `BuildInfoFileWriteAction.execute` against BuildInfoFileWriteAction.java:97-180: the parse
  failure text and `SOURCE_INPUT_IO_EXCEPTION`; the callback failure text with
  `EvalException.toString()` (message with stack) and `NON_ACTION_EXECUTION_FAILURE`; the
  `Dict.cast` texts (Dict.java:659-679) inside "expected to return dict of strings to
  strings"; the action's own semantics; the default print handler writing to stderr
  (StarlarkThread.java:366); substitutions in the returned dict's order; template expansion
  not executable. The order of the dict handed to the callback follows Java: `parseValues`'
  `HashMap`, then `putAll` into a table presized to `tableSizeFor(n / 0.75 + 1)`, which never
  resizes for n entries.

## Open items that do not block

1. Round-1 open items 1, 2 and 6 (fail-fast handler, `get_output_group`, the
   `from_utf8_lossy` note) are unchanged, as the response says.
2. `java_hash_map_iteration_order` does not model treeified buckets (marked).
3. An interrupted callback becomes an `ActionExecutionException("interrupted")`; Java lets
   `InterruptedException` out of `execute`. Check that the caller treats it as an interrupt.

## Verdict

**PASS**, bound to 5f68157. No blocking finding. R1 is S2 and is expected to be fixed in the
next batch; until then "execution under `strip` fails loudly" holds for Starlark and plain
spawn actions and for Java, C++ and parameter-file actions without colliding inputs.
