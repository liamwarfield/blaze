# W3 loading: shared design (blaze_config, blaze_packages, blaze_sky_loading, blaze_query_engine, blaze_query)

Goal: given external repositories already on disk, Blaze loads packages exactly like Bazel 9.3.2
and `query` output (label_kind, streamed_jsonproto, and deps()) equals the oracle's recordings.
Read `AGENTS.md` and `PORTING.md` first. This file fixes the decisions that cross file and crate
boundaries; inside your own files, port the Java literally.

## Process

- Agents do **not** run Cargo. The lead builds and writes diagnostics with
  `scripts/w3/build-errors <dir>`: `<dir>/SUMMARY.txt`, `<dir>/<crate>.txt`, and
  `<dir>/files/<path with / replaced by __>.txt` (errors first). Read the files for your paths.
- Edit only files you own (ownership table below). Need something from a file you do not own?
  Add it to your report under "requests" with the exact signature; meanwhile call
  `unported!("Class.method")` or write a private helper marked `// PORT: move to <file>`.
- Every ported item: `// Java: com/google/devtools/build/lib/<path>.java:<line> Class.method`.
  Unported paths: `blaze_util::unported!("Class.method")` (never a guessed value).
- `rustfmt --edition 2024` every file you change. Starlark-visible names, error texts and output
  ordering are Bazel's, character for character.
- Tests: port the Java JUnit tests for what you port (`tests/` or `#[cfg(test)]`, snake_case Java
  names); `#[ignore = "needs ..."]` for those that need missing pieces.

## Core model (blaze_packages)

- **Types.** `Type` is a closed Rust `enum` with one variant per Java constant (`Type.INTEGER`,
  `STRING`, `BOOLEAN`, `Types.STRING_LIST`, `STRING_DICT`, `BuildType.LABEL`, `NODEP_LABEL`,
  `LABEL_LIST`, `LABEL_KEYED_STRING_DICT`, `OUTPUT`, `LICENSE`, `TRISTATE`, ...). Java identity
  comparisons (`type == BuildType.LABEL`) become `==` on the enum. Converted attribute values are
  plain Rust data in `AttrValue` (an enum: `Int(i32)`, `String`, `Bool`, `Label(Label)`,
  `StringList`, `LabelList`, `StringDict(IndexMap)`, `LabelKeyedStringDict`, `License`,
  `TriState`, ..., plus `SelectorList`, `ComputedDefault`, `LateBound` where Java stores those
  objects). `Type::convert(&self, x: starlark Value, what: &dyn Display, LabelConverter)` returns
  `Result<AttrValue, ConversionException>` with Java's messages.
- **Send + Sync.** Everything Skyframe stores (`Package`, targets, `RuleClass`, `Attribute`,
  values) is immutable after construction and `Send + Sync`. Shared things are `Arc`.
- **Targets.** `enum Target { Rule(Arc<Rule>), InputFile(..), OutputFile(..), PackageGroup(..),
  EnvironmentGroup(..) }`. Targets do not point back to `Package` (no `Arc` cycles): they hold an
  `Arc<PackageMetadata>` (Java `Package.Metadata`: package id, build file label/path, repo
  mapping, source root, ...) and, where Java needs it, `Arc<PackageDeclarations>`
  (`Package.Declarations`: default visibility, testonly, deprecation, features, ...). A caller
  that needs the whole `Package` looks it up by `PackageIdentifier`.
- **Locations** are `blaze_starlark::Location` (`file:line:col`); `file` is the absolute path
  string Bazel prints.
- **Labels** are `blaze_cmdline::Label`; Starlark label values `blaze_starlark::label::LabelValue`.
- **Events** recorded during loading are `blaze_events` events stored in the package and replayed
  by PACKAGE, as Java does.

## Starlark integration

- One `Module` per evaluation unit (a BUILD file, a .bzl top level, REPO.bazel, a builtins .bzl).
  Values that leave it are frozen. `blaze_starlark::eval` has compile / exec_module /
  call_frozen.
- Phase state lives in `blaze_starlark::ThreadContext` payloads:
  - BUILD and macros: the package builder (`Package::Builder` behind the payload type defined in
    `package/`), Java `TargetDefinitionContext.fromOrFail(thread, ...)`.
  - .bzl top level: `BzlInitThreadContext` (Java packages/BzlInitThreadContext.java) with the
    file's `BazelModuleContext` (label, transitive digest, repo mapping, loads) plus a map
    `starlark file name -> Arc<BazelModuleContext>` for every module reachable through loads, so
    natives can find the module of the innermost enclosing Starlark function from the call stack
    (Java `BazelModuleContext.ofInnermostBzlOrThrow(thread)`).
  - The frozen module of each .bzl carries its `BazelModuleContext` as the module extra value
    (`Module::set_extra_value`, `FrozenModule::extra_value`).
- **Export.** Java exports `StarlarkExportable`s from a post-assignment hook; starlark-rust calls
  `StarlarkValue::export_as(name, eval)` on top-level assignment. Use it for rule classes,
  providers, aspects, macros, transitions.
- **Rule classes from rule().** `rule()` returns a complex Starlark value
  (`StarlarkRuleFunction<V>`) holding the attribute descriptors and callables as `V`. The
  immutable `Arc<RuleClass>` is built when the value is exported/frozen. Rules are only ever
  instantiated from frozen values (BUILD files and macros call functions of loaded, frozen .bzl
  modules), so starlark callables inside `RuleClass`/`Attribute` (implementation, computed
  default functions, initializers, outputs functions, transitions) are stored as
  `StarlarkCallable` = a `FrozenValue` handle. BZL_LOAD keeps every frozen .bzl heap alive for the
  life of the process (`blaze_sky_loading` keep-alive list; single build per process), which makes
  those handles valid; mark with `// PORT: keep-alive`.
- Natives are `#[starlark_module]` functions registered through
  `blaze_starlark::env::NativeRegistry` per `FileKind`. Parameter names, positional/named rules
  and defaults follow the Java `@Param`s.

## Skyframe (blaze_sky_loading)

- `blaze_cmdline` and `blaze_vfs` do not depend on `blaze_skyframe`, so keys are newtypes in
  `blaze_sky_loading` (`PackageKey(PackageIdentifier)`, `BzlLoadKey::{ForBuild, ForBzlmod,
  ForBuiltins}`, `FileStateKey(RootedPath)`, ...). `Debug` prints what Java's `toString` prints.
- Functions are blocking: `env.get_value(k)?` returns the dependency or `None` (missing because
  of an error). Keep Java's structure (getValue / valuesMissing / getValueOrThrow).
- **External repos seam.** `blaze_sky_loading::repos::ExternalRepos` (a trait) supplies
  `REPOSITORY_MAPPING` (`RepositoryMappingValue`: mapping + associated module name/version, or
  not found) and `REPOSITORY_DIRECTORY` (`RepositoryDirectoryValue`: success with a path, or a
  failure message). For W3 the implementation is `OracleRepos`, read from a JSON file produced by
  `scripts/w3/oracle-repos` (oracle `mod dump_repo_mapping` + `mod graph`) and the repo
  directories in the oracle's output bases. The Bzlmod lead later implements the same trait with
  real resolution and fetching.

## Ownership

| Owner | Files |
| --- | --- |
| A1 (types, attributes, rule classes) | `blaze_packages/src/{types,attribute,rule_class}/**` |
| A2 (package, rules, targets, macros) | `blaze_packages/src/{package,target,macros}/**`, `blaze_packages/src/exceptions.rs` |
| B (.bzl rule-definition builtins) | `blaze_packages/src/starlark_defs/**`, `blaze_packages/src/bzl_context.rs`, `blaze_config/**` |
| C (BUILD loading, natives, native rules) | `blaze_packages/src/{build,native_rules}/**` |
| D (Skyframe loading, builtins injection, REPO.bazel) | `blaze_packages/src/{builtins,repo_file}/**`, `blaze_sky_loading/**`, `scripts/w3/oracle-repos` |
| E (query) | `blaze_query_engine/**`, `blaze_query/**` |
| lead | `blaze_packages/src/lib.rs`, `Cargo.toml`s, this file, `scripts/w3/*` not listed above |

Ask the lead for module declarations in `lib.rs` and for new dependencies.

## Environment composition contract

- B exports `starlark_defs::register_bzl_toplevels(&mut GlobalsBuilder)`: every fixed .bzl
  toplevel it ports (Java `StarlarkGlobalsImpl.getFixedBzlToplevels` minus `native`): rule, attr,
  provider, aspect, exec_group, subrule, macro, transition, config, config_common,
  platform_common, configuration_field, visibility, DefaultInfo, OutputGroupInfo, ... .
- C exports `build::register_build_globals(&mut GlobalsBuilder)` (BUILD-only functions: package,
  glob, licenses, exports_files, package_group, existing_rule(s), ...), `build::register_select`
  (`select`, used by BUILD and .bzl), `build::register_native_module_functions` (the non-rule
  members of `native`), and `native_rules::RuleClassProvider` (Java ConfiguredRuleClassProvider
  subset: native rule classes by name, in registration order, plus their `BuiltinRuleFunction`s).
- D composes the environments per file kind with `@_builtins` injection
  (`builtins/`: Java BazelStarlarkEnvironment, StarlarkBuiltinsFunction, BuiltinsInternalModule,
  AutoloadSymbols) and runs BZL_LOAD / PACKAGE. `bzl_context.rs` holds `BazelModuleContext`
  (registered per compiled file name) and `BzlInitThreadContext` (the .bzl payload).
- Payload borrows: `TargetDefinitionContext::from_or_fail` and `BzlInitThreadContext::from_or_fail`
  return a `RefMut`; drop it before calling back into Starlark.
