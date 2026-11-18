# Native Starlark/build API inventory for brotli and re2

Written 2026-11-17, against Bazel 9.3.2 (commit 5951e93c) and the corpus commits in
`corpus/projects.json` (brotli @3dce8720, re2 @2da00568). The machine-readable table is
`docs/research/native-api.tsv`. Tools: `scripts/inventory/` (each prints usage with `help`).
Java paths below are relative to `src/main/java` of the pinned source.

## Summary

- **What Bazel loads.** brotli's `build //...` loads 658 Starlark files (98k lines). re2's
  `test //:all //python:all` loads 724 files (112k lines). Together that is 740 distinct files
  (113k lines): 347 `.bzl` loaded from BUILD files, 59 `.bzl` loaded only by module extensions
  and repository rules, 15 `builtins_bzl` files, 1 bootstrap `.bzl`
  (`@bazel_tools//tools/build_defs/repo:http.bzl`), 131 BUILD files, 170 `MODULE.bazel` files
  and 17 `REPO.bazel` files. These counts come from the oracle's own Skyframe graph, not a guess.
- **Native surface in use.** 410 distinct native symbols:
  - 52 global functions and objects (BUILD, .bzl, MODULE.bazel and REPO.bazel).
  - 14 native rule classes.
  - 344 members on native types: 208 methods and 136 fields. 74 of these are matched by member
    name only (marked `*` below and "receiver inferred" in the TSV).
  - 31 more member names where the receiver could not be resolved (`?` rows).

  Bazel annotates 917 `@StarlarkMethod`s in total, so these projects touch about 40% of them.
- **Everything outside the native surface is Starlark.** All 20 repository rules that run
  are Starlark, `http_archive` included. So are all 16 module extensions and every rule that
  gets analyzed, apart from the 14 native rule classes. C++ is in rules_cc 0.2.17 Starlark
  (37.6k lines, 29.3k of them loaded). Under it sits a private native layer:
  - `cc_internal`: 33 methods.
  - `cpp` fragment: 24 members.
  - `FeatureConfiguration` and `CcToolchainFeatures`.
  - `HeaderInfo`: 8 fields.
  - `cc_common`: 15 wrapper members.

  That layer reaches about 12k lines of Java in `rules/cpp` (17k lines in the whole package).
  Python is rules_python 1.7 Starlark plus 14 `py_builtins` methods (420 lines of Java).
- **brotli needs almost all of it.** In the TSV, 98% of rows say `needed_by=both`. brotli loads
  nearly the same `.bzl` closure as re2. Registered toolchains force Bazel to load the packages
  that declare them, and those packages pull in rules_java, rules_python, rules_swift,
  rules_kotlin, apple_support and protobuf. brotli fetches 66 repositories, 37 of them rules_java
  JDK toolchain-config repos, and configures 205 `toolchain` targets for a 13-target C build.
  The difference between the projects is in what gets **executed**: which rule implementations
  are analyzed and which actions run. See "Implementation order".

## Method

All runs used copies of the corpus checkouts in `~/.cache/blaze/work/inventory/<name>`, a
separate output root (`~/.cache/blaze/oracle-out/ur-inventory`), `--nohome_rc --nosystem_rc` and
the shared repository cache, under `systemd-run` with a 12 GB cap.

1. `scripts/inventory/oracle-run <project>` runs the real command: brotli built 53 actions;
   re2 ran 716 actions and 21 of 21 tests passed. In the **same server** it then runs
   `bazel dump --skyframe=deps` filtered to `BZL_LOAD|PACKAGE|REPOSITORY_DIRECTORY|
   SINGLE_EXTENSION_EVAL|MODULE_FILE|...`. The Skyframe keys are the exact set the oracle
   evaluated:
   - `BZL_LOAD:KeyForBuild`, `KeyForBzlmod`, `KeyForBuiltins` and `KeyForBzlmodBootstrap`.
   - `PACKAGE`, `MODULE_FILE`, `REPOSITORY_DIRECTORY` and `SINGLE_EXTENSION_EVAL`.

   `dump --rules` gives the configured-target count per rule class. The script also runs
   `query buildfiles(deps(...))`, `cquery deps(...)` followed by `buildfiles` of its labels,
   `query <pkg>:*` with `label_kind`, `mod graph`, `info build-language` (the native rule
   classes) and `mod show_repo` for every fetched repo (repo to repository rule). The query
   outputs are cross-checks; the Skyframe dump is the source of truth. `query deps()` on re2
   needs `--keep_going`: it walks every `select()` branch, including `@libpfm`, which needs
   the network.
2. `scripts/inventory/extract.py files|uses <project>` maps each key to its file on disk
   (`output_base/external/<canonical>`, `builtins_bzl`, the workspace, or `MODULE.bazel` from
   the repository cache through `registryFileHashes` in the lockfile). It parses every file with
   Python's `ast` (all 740 parse) and records:
   - every free global name, which is a predeclared symbol;
   - every `x.member` access, with a small flow-insensitive type inference:
     - the parameter `ctx` is typed by role: the `implementation=` of `rule`/`aspect` gives the
       rule `ctx`, `repository_rule` gives `repository_ctx`, `module_extension` gives
       `module_ctx`;
     - return types come from the Java signature of each `@StarlarkMethod` (`ctx.actions`
       returns `actions`, `actions.args()` returns `Args`);
     - local and module-level assignments, `load()` aliases across files,
       `ctx.fragments.<f>`, `x[DefaultInfo]` and parameter-name heuristics (`rctx`,
       `feature_configuration`, `label`, `file`...).
3. `scripts/inventory/java_index.py` indexes all 917 `@StarlarkMethod` and 161
   `@StarlarkBuiltin` annotations, with the declaring type, structField, Java method name, return
   type and the concrete implementing method found in subclasses. `java_impl` in the TSV is the
   implementation when one was found, otherwise the annotation.
4. `scripts/inventory/report.py` classifies each use:
   - core: starlark-rust language;
   - native: receiver resolved to a native type;
   - native-inferred: receiver unknown, but exactly one native type that is reachable in these
     projects has the member;
   - ambiguous;
   - starlark: Starlark struct or provider fields;
   - dynamic: `ctx.attr.<x>`, `ctx.file.<x>` and the like.

   It then aggregates into the TSV. "Reachable" types start from the globals and contexts and
   are closed over the return types of resolved native calls. Without that rule, members of
   Starlark providers such as rules_cc's `CcCompilationContextInfo.headers` would wrongly match
   legacy Java types like `CompilationContext`.

Use sites per project, counted once per (file, line, receiver) and with files that are loaded
under two key kinds counted once:

| project | native | native-inferred | ambiguous | core | starlark | dynamic |
|---|---|---|---|---|---|---|
| brotli | 9,845 | 451 | 499 | 4,402 | 5,157 | 960 |
| re2 | 10,348 | 455 | 509 | 4,489 | 5,219 | 1,005 |

**Limits.** This is a static inventory of loaded code. It counts call sites, not executions:
a symbol in a loaded rules_java file may never run for these projects. Receiver typing is
heuristic. The `?` rows (about 500 sites) are mostly `.name`, `.path`, `.label`, `.files`,
`.basename` and `.merge`, which are either a native field (Label, File, path, runfiles) or a
Starlark struct field. Getting this right takes real type information. Some native behaviour
is not a method call and does not appear as a row:
- indexing a Target by provider (`dep[CcInfo]`, `CcInfo in dep`);
- `ctx.toolchains["//t:type"]`;
- `+` and `|` on `select`, depset and dict;
- `str(Label)`, plus equality and hashing of File and Label;
- calling a provider (`ToolchainInfo(...)`);
- `getattr` and `hasattr` with string names.

## Files loaded

| kind | brotli files / lines | re2 files / lines |
|---|---|---|
| BUILD | 89 / 10,654 | 130 / 21,962 |
| .bzl from BUILD (`KeyForBuild`) | 322 / 64,902 | 349 / 68,463 |
| .bzl only from extensions or repo rules (`KeyForBzlmod`) | 58 / 11,935 | 57 / 10,672 |
| builtins_bzl (`KeyForBuiltins`) | 15 / 1,545 | 15 / 1,545 |
| bootstrap (`http.bzl`) | 1 / 672 | 1 / 672 |
| MODULE.bazel | 158 / 8,510 | 155 / 8,820 |
| REPO.bazel | 15 | 17 |

Loaded Starlark by repository (re2; brotli is the same minus abseil, googletest, benchmark,
pybind11 and abseil-py, and has fewer rules_python files):

| repo | files | lines |
|---|---|---|
| rules_cc | 119 | 29,331 |
| rules_python | 111 | 20,763 |
| rules_java | 103 | 11,818 |
| rules_swift | 30 | 8,986 |
| abseil-cpp | 24 | 8,554 |
| rules_license | 6 | 4,465 |
| protobuf | 31 | 4,361 |
| bazel_tools | 39 | 3,781 |
| bazel_skylib | 20 | 3,082 |
| rules_kotlin | 16 | 1,816 |
| _builtins | 15 | 1,545 |
| apple_support | 10 | 1,136 |
| rules_shell | 13 | 834 |
| others (abseil-py, main, platforms, bazel_features, googletest, pybind11_bazel, google_benchmark) | 32 | 2,842 |

Only 15 of the 29 files in `builtins_bzl` load in 9.3.2:
- `exports.bzl` (top level, `common/` and `bazel/`) and `common/util.bzl`;
- `cc/cc_common_bazel.bzl` and `cc/cc_helper_internal.bzl`;
- `java/java_common.bzl` and `python/py_internal.bzl`;
- `xcode/providers.bzl`, `builtin_exec_platforms.bzl` and the five `objc/` files.

`cc_library.bzl`, `cc_binary.bzl`, `compile.bzl` and the other old C++ rule bodies are dead
code at this pin, because rules_cc 0.2.x defines the C++ rules.

## Native rules (BUILD files and `native.*`)

Static call sites in the BUILD files that load, against configured targets from `dump --rules`.
Alias targets are counted under their actual rule by `dump`.

| rule | static calls (brotli / re2) | configured (brotli / re2) | Java |
|---|---|---|---|
| alias | 176 / 183 | (n/a) | rules/Alias.java:37 |
| config_setting | 124 / 166 | 107 / 258 | rules/config/ConfigRuleClasses.java:132 |
| toolchain | 110 / 110 | 205 / 615 | rules/platform/ToolchainRule.java:35 |
| filegroup | 86 / 97 | 28 / 47 | rules/filegroup/FilegroupRule.java:77 |
| constraint_value | 59 / 59 | 49 / 95 | rules/platform/ConstraintValueRule.java:34 |
| toolchain_type | 26 / 26 | 25 / 55 | rules/ToolchainType.java:79 |
| constraint_setting | 8 / 8 | 4 / 4 | rules/platform/ConstraintSettingRule.java:34 |
| label_flag | 4 / 8 | 0 / 0 | rules/LabelBuildSettings.java:120 |
| platform | 2 / 7 | 1 / 1 | rules/platform/PlatformRule.java:39 |
| genrule | 2 / 3 | 0 / 0 | bazel/rules/genrule/BazelGenRuleRule.java:54 |
| test_suite | 2 / 3 | 0 / 0 | rules/test/TestSuiteRule.java:116 |
| cc_libc_top_alias | 1 / 1 | 0 / 0 | rules/cpp/CcLibcTopAlias.java:21 |
| java_plugins_flag_alias | 1 / 1 | 0 / 0 | rules/java/JavaPluginsFlagAliasRule.java:65 |
| starlark_doc_extract | 1 / 1 | 0 / 0 | rules/starlarkdocextract/StarlarkDocExtractRule.java:134 |

BUILD-file functions: `glob` (59 sites), `package` (65), `licenses` (49), `exports_files` (21),
`package_group` (4), `select` (142, BUILD and .bzl), and `module_version` (1, re2 only). In
macros: `native.package_name`, `glob`, `module_name`, `package_relative_label`,
`existing_rule`, `repository_name` and `exports_files`.

The `cc_*`, `java_*` and `objc_*` names in `bazel info build-language` are `EmptyRule`
placeholders. When called unloaded they hit the `_removed_rule_failure` stub in
`builtins_bzl/bazel/exports.bzl`, so every analyzed `cc_library`, `cc_toolchain_alias` and
similar target comes from rules_cc's Starlark. Starlark rule classes that get configured:

- brotli: `cc_library` (13), `cc_binary` (2), `cc_toolchain`, `cc_toolchain_config`,
  `cc_toolchain_alias`, `bazel_cc_build_info`, `xcode_config` (apple_support), `compiler_flag`,
  `string_flag`/`bool_flag` (skylib), `_license`, `_license_kind`, `_python_version_flag`.
- re2 also: `cc_test` (20), `cc_library` (170), `py_library` (21), `py_test`, `py_runtime`,
  `py_runtime_pair`, `py_cc_toolchain`, `py_exec_tools_toolchain`, `py_interpreter_program`,
  `current_interpreter_executable`, `current_py_cc_headers`, `sh_binary` (2), `sh_library`,
  `sh_toolchain`, `default_test_runner` (2, the Bazel 9 test-runner toolchain), `empty_toolchain`,
  `_copy_file`, `root_symlinks`, `is_windows`, `bool_setting`, `string_list_flag`.

## Repository rules and module extensions

There are no native repository rules. `bazel_tools` is the only builtin repo (embedded). Every
other repo comes from a Starlark rule, as `mod show_repo` reports:

| repository rule | repos (brotli / re2) |
|---|---|
| `@rules_java//toolchains:remote_java_repository.bzl%_toolchain_config` | 37 / 37 |
| `@bazel_tools//tools/build_defs/repo:http.bzl%http_archive` | 12 / 18 |
| `@rules_python//python/private:python_repository.bzl%python_repository` | 0 / 1 (CPython 3.11 download) |
| one repo each, in both projects | 17 |

The 17 one-repo rules:
- rules_cc: `cc_autoconf`, `cc_autoconf_toolchains`, `_compatibility_proxy_repo_rule`;
- rules_java: `_local_java_repository_rule`, `_compatibility_proxy_repo_rule`;
- rules_python: `hub_repo`, `internal_config_repo`, `uv_toolchains_repo`;
- `sh_config`, `swift_autoconfiguration`, `kotlin_capabilities_repository`;
- `host_platform_repo`, `version_repo`, `globals_repo`;
- `apple_cc_autoconf_toolchains`, `xcode_autoconf`, `winsdk_configure`.

Module extensions evaluated, 15 in brotli plus 1 in re2. All are Starlark:
- `apple_cc_configure_extension`, `version_extension` (bazel_features), `host_platform`;
- `xcode_configure_extension` and the innate `winsdk_configure` (a `use_repo_rule` in
  `@bazel_tools//:MODULE.bazel`);
- rules_cc `cc_configure_extension` and `compatibility_proxy`;
- rules_java `toolchains` and `compatibility_proxy`;
- `rules_kotlin_extensions`;
- rules_python `config`, `python` and `uv`;
- `sh_configure`, swift `non_module_deps`;
- re2 only: pybind11 `internal_configure_extension`.

The native work is in `repository_ctx` and `module_ctx`, the Bzlmod resolver, the downloader and
the repository cache. Note that brotli's tracked `MODULE.bazel.lock` already holds some
extension results; with a lockfile present, Bazel may skip the implementation of a
non-reproducible extension. Blaze has to handle both cases.

## Native surface by area

Format: `symbol(sites)`. A `.` suffix marks a field. `*` marks a receiver inferred from the
member name. `[re2]` marks re2 only. Everything else appears in both projects.

**Bzlmod and repositories**
- MODULE.bazel (9): bazel_dep(889), use_repo(278), use_extension(228), module(170),
  register_toolchains(104), single_version_override(17), local_path_override(14),
  archive_override(12), use_repo_rule(8). Extension proxies `ext.tag(...)` dispatch through
  `TagCallable`, in bazel/bzlmod/ModuleFileGlobals.java.
- .bzl globals: repository_rule(32), module_extension(16), tag_class(11).
- repository_ctx (18): attr.(150), file(74), os.(56), path(55), execute(33), template(18),
  symlink(12), which(12), delete(11), name.(7), download(6), download_and_extract(6), read(6),
  report_progress(6), repo_metadata(4), patch(3), getenv(1), watch(1). In
  bazel/repository/starlark/StarlarkRepositoryContext.java and StarlarkBaseExternalContext.java.
- module_ctx (7): extension_metadata(11), modules.(10), read(3), download(2), path(2), getenv(1),
  os.(1). bazel_module: version., tags., is_root.
- path (5): exists., dirname., get_child, readdir, realpath.
- exec_result: stdout., stderr., return_code.
- repository_os: environ., arch., name.
- REPO.bazel: `repo()` (packages/RepoFileGlobals.java).

**Defining rules (.bzl top level)**
- Globals: attr(818), depset(536), struct(332), Label(177), provider(156), select(142),
  DefaultInfo(132), native(124), rule(110), platform_common(45), OutputGroupInfo(33),
  visibility(32), config(27), config_common(27), configuration_field(27),
  PackageSpecificationInfo(9), testing(9), RunEnvironmentInfo(6), coverage_common(6),
  exec_group(6), aspect(5), transition(4), subrule(3), AnalysisFailureInfo(2), json(2),
  AnalysisTestResultInfo(1), analysis_test_transition(1), macro(1).
- attr (14): label(241), string(188), label_list(131), string_list(131), bool(65),
  string_dict(32), int(12), license(11), string_list_dict(11), output(3),
  label_keyed_string_dict(2), string_keyed_label_dict(2), int_list(1), output_list(1).
- config (7): exec, string, bool, string_list, int, none, target. These are build settings and
  transitions.
- config_common: toolchain_type(10), FeatureFlagInfo.(17).
- platform_common: ToolchainInfo.(26), TemplateVariableInfo.(12), ConstraintValueInfo.(7).

**Analysis: rule context and actions**
- ctx (28): attr.(675), actions.(226), label.(177), file.(78), files.(76), runfiles(70)*,
  fragments.(65), configuration.(37), toolchains.(36), workspace_name.(25), features.(22)*,
  executable.(19), outputs.(19)*, disabled_features.(14), bin_dir.(13), rule.(11),
  expand_location(10), build_setting_value.(8), var.(8), target_platform_has_constraint(7),
  genfiles_dir.(7), expand_make_variables(5), version_file.(4), coverage_instrumented(3),
  tokenize(3), aspect_ids.(3), exec_groups.(3), info_file.(2). subrule_ctx: actions., label.,
  fragments.
- actions (14): declare_file(87), args(54), run(39), symlink(21), write(18),
  declare_shareable_artifact(11), expand_template(7), declare_directory(6), declare_symlink(5),
  run_shell(4), declare_shareable_directory(2), template_dict(2), transform_info_file(2),
  transform_version_file(2). TemplateDict.add_joined.
- Args (5): add(162), add_all(77), use_param_file(18), set_param_file_format(15), add_joined(9).
- File (9): path., short_path., extension., owner., basename., is_directory., is_source.,
  dirname., root. (root.path 24).
- Label (7): name., package., workspace_root., workspace_name., repo_name., relative,
  same_package_label.
- depset.to_list(128).
- runfiles (6): merge, merge_all, files., root_symlinks., symlinks., empty_filenames.
- DefaultInfo: files., files_to_run., default_runfiles., data_runfiles.
- FilesToRunProvider.executable.
- configuration (8): coverage_enabled., is_tool_configuration, stamp_binaries, runfiles_enabled,
  is_sibling_repository_layout, host_path_separator., has_separate_genfiles_directory,
  default_shell_env.
- rule_attributes (`ctx.rule` in aspects): attr., kind.

**Tests and coverage**: testing.ExecutionInfo(7), testing.TestEnvironment(2),
RunEnvironmentInfo.environment, coverage_common.instrumented_files_info(6),
analysis_test_transition, AnalysisTestResultInfo.

**C++ (native layer under rules_cc)**
- cc_internal (33, rules/cpp/CcStarlarkInternal.java):
  - check_private_api(45), get_artifact_name_for_category(34), freeze(32),
    combine_cc_toolchain_variables(12), actions2ctx_cheat(10), cc_toolchain_variables(8),
    create_cc_compile_action(7), dynamic_library_symlink(5),
    intern_string_sequence_variable_value(5), is_tree_artifact(5), wrap_link_actions(4);
  - compute_output_name_prefix_dir, create_header_info, declare_other_output_file,
    dynamic_library_soname, dynamic_library_symlink2, absolute_symlink, cc_toolchain_features,
    check_toplevel;
  - collect_per_file_lto_backend_opts, create_cc_compile_action_template,
    create_header_info_with_deps, create_lto_backend_action, create_lto_backend_action_template,
    declare_compile_output_file;
  - exec_os, expand_and_tokenize, get_artifact_name_extension_for_category, get_link_args,
    intern_seq, per_file_copts, rule_class, solib_symlink_action.
- cc_common, as wrapped by `builtins_bzl/common/cc/cc_common_bazel.bzl` (15 used; rules_cc
  reaches them through `native_cc_common.bzl`): action_is_enabled, get_environment_variables,
  get_execution_requirements, get_tool_for_action, empty_variables,
  get_memory_inefficient_command_line, get_tool_requirement_for_action,
  legacy_cc_flags_make_variable_do_not_use, do_not_use_tools_cpp_compiler_present,
  check_experimental_cc_shared_library, incompatible_disable_objc_library_transition,
  add_go_exec_groups_to_binary_rules, implementation_deps_allowed_by_allowlist,
  internal_DO_NOT_USE (which returns `cc_internal`), and is_enabled. Native in
  rules/cpp/CcModule.java. A few other `cc_common.*` rows in the TSV (link,
  create_linking_context, create_cc_toolchain_config_info...) are calls into rules_cc's
  *Starlark* `cc_common`, mis-typed through the `cc_compatibility_proxy` aliasing.
- cpp fragment (24, rules/cpp/CppConfiguration.java):
  - force_pic, compilation_mode, dynamic_mode, strip_opts, share_native_deps;
  - fdo_path, fdo_instrument, cs_fdo_path, cs_fdo_instrument, proto_profile,
    propeller_optimize_absolute_cc_profile, propeller_optimize_absolute_ld_profile,
    _fdo_prefetch_hints_label.;
  - experimental_cc_implementation_deps, experimental_cpp_modules,
    experimental_link_static_libraries_once, fission_active_for_current_compilation_mode,
    disable_nocopts, _dont_enable_host_nonhost.;
  - minimum_os_version, apple_generate_dsym., objc_generate_linkmap., objc_should_strip_binary.,
    custom_malloc..
- FeatureConfiguration.is_enabled(87) and is_requested(4)
  (rules/cpp/FeatureConfigurationForStarlark.java).
- CcToolchainFeatures.configure_features and default_features_and_action_configs.
- HeaderInfo (8 fields).

**Python**: `py_internal`, wrapped by `builtins_bzl/common/python/py_internal.bzl` and
implemented in rules/python/PyBuiltins.java (14 used):
- get_label_repo_runfiles_path, copy_without_caching, create_repo_mapping_manifest,
  declare_constant_metadata_file, expand_location_and_make_variables;
- is_bzlmod_enabled, is_singleton_depset,
  merge_runfiles_with_generated_inits_empty_files_supplier, add_py_extra_pseudo_action,
  are_action_listeners_enabled, create_sources_only_manifest, get_current_os_name,
  get_rule_name, regex_match.

Fragments: py.build_python_zip, py.default_to_explicit_init_py, bazel_py.python_path,
bazel_py.python_import_all_repositories. rules_python also uses `transition`, `config.*`,
`json` and `RunEnvironmentInfo`.

**Java and Apple, loaded but mostly not executed here**:
- java_common (15) and the java fragment (15);
- apple_common: platform., dotted_version, XcodeVersionConfig., apple_toolchain,
  apple_host_system_env, target_apple_env;
- the apple fragment (8), objc (2), apple_platform (3) and DottedVersion.compare_to.

`xcode_config` *is* analyzed in both projects, so a handful of the apple members do run.

**Builtins plumbing**: `_builtins.internal.{cc_common,cc_internal,apple_common,py_builtins,
java_common_internal_do_not_use}` and `_builtins.toplevel.{native,PackageSpecificationInfo,
proto_common_do_not_use,exec_transition}` (packages/BuiltinsInternalModule.java).
`proto_common_do_not_use.incompatible_enable_proto_toolchain_resolution` and the `proto`
fragment (3 fields) are read by protobuf's .bzl at load time.

## How much is Starlark and how much is Java

| area | Starlark (loaded / repo total) | native Java behind it |
|---|---|---|
| C++ rules | rules_cc 0.2.17: 119 files and 29,331 lines loaded, of 164 files and 37,603 lines; builtins cc: 2 files, 150 lines | 88 native symbols (cc_internal 33, cpp 24, cc_common 20 rows, FeatureConfiguration 2, HeaderInfo 8, CcToolchainFeatures 1). The directly called classes come to 6,963 lines; the action classes they build are another ~6k (sizes below). `rules/cpp` totals 16,955 lines. |
| Python rules | rules_python 1.7: 111 files and 20,763 lines loaded, of 309 files and 50,342 lines; builtins python: 163 lines | 18 symbols: PyBuiltins.java (420 lines) plus the py and bazel_py fragments (309 lines) |
| Test runner | `@bazel_tools//tools/test`: test-setup.sh (377 lines, shell, runs as is), generate-xml.sh (130), collect_coverage.sh (270, coverage only), default_test_toolchain.bzl (`default_test_runner`) | TestRunnerAction (1,401), StandaloneTestStrategy (902), TestActionBuilder (520), TestStrategy (506), TestConfiguration (578), TestPolicy (103) |

The C++ action classes in rules/cpp, in lines:

| class | lines |
|---|---|
| CppCompileAction | 2,125 |
| CcToolchainFeatures | 1,352 |
| CcStarlarkInternal | 1,298 |
| CcToolchainVariables | 1,064 |
| CppOptions | 1,026 |
| CppConfiguration | 968 |
| CcCompilationContext | 925 |
| CppCompileActionBuilder | 664 |
| CppCompileActionTemplate | 408 |
| SolibSymlinkAction | 336 |
| LtoBackendAction | 324 |
| HeaderDiscovery | 306 |
| LinkCommandLine | 229 |
| CompileCommandLine | 228 |
| CcModule | 216 |

The C++ compile action is native: `create_cc_compile_action` builds a `CppCompileAction`, with
include scanning, `.d`-file header discovery and the `parse_headers`/`layering_check` features
that re2's `.bazelrc` turns on. The feature and variable engine is native too
(`CcToolchainFeatures` with `CcToolchainVariables`). Linking is mostly Starlark in rules_cc:
`finalize_link_action.bzl` calls `ctx.actions.run` with `cc_internal.get_link_args`.

## Implementation order

The steps follow what the oracle executes, not just what it loads.

**Step 1: brotli loading phase.** brotli cannot load until all of this is present, because the
same files load for both projects.
1. builtins_bzl injection: `_builtins.internal/toplevel`, then the 15 builtins files that
   override `cc_common`, `py_internal`, `apple_common` and `java_common`, plus the
   `_removed_rule_failure` stubs.
2. Bzlmod: the 9 MODULE.bazel globals with tag proxies, resolution across 158 module files,
   the lockfile, `module_extension`, `tag_class`, `module_ctx` (7 members) and `bazel_module`.
3. Repositories: `repository_rule` plus the `repository_ctx` members in the table. `execute`,
   `which`, `os.environ`, `file`, `template`, `symlink`, `path`, `read`, `delete` and
   `download_and_extract` cover the 20 rules. The local repos `local_jdk`, `local_config_cc`,
   `local_config_xcode` and `local_config_sh` probe the host with `execute` and `which`.
4. .bzl definition globals: `rule`, `provider`, `attr.*` (14), `depset`, `struct`, `Label`,
   `select`, `visibility`, `configuration_field`, `config.*` (7), `config_common`,
   `platform_common`, `exec_group`, `subrule`, `aspect`, `transition`, `macro`, `native.*` (7).
5. BUILD: `package`, `glob`, `licenses`, `exports_files`, `package_group`, and the native
   rules. In order of count: alias, config_setting, toolchain, filegroup, constraint_value,
   toolchain_type, constraint_setting, label_flag, platform, genrule, test_suite,
   cc_libc_top_alias, java_plugins_flag_alias, starlark_doc_extract.

**Step 2: brotli analysis and execution.**
1. Toolchain resolution: 205 `toolchain` targets, constraints and platforms.
2. ctx and actions core: `ctx.attr/file/files/executable/label/actions/fragments/
   configuration/toolchains/features/disabled_features/bin_dir/runfiles/expand_location/var/
   version_file/info_file`; `actions.declare_file/args/run/write/symlink/expand_template/
   transform_*_file`; Args (5); File; Label; depset; runfiles; DefaultInfo; OutputGroupInfo.
3. C++: `cpp` fragment, `cc_internal` (compile path first: `cc_toolchain_variables`,
   `combine_cc_toolchain_variables`, `create_cc_compile_action`,
   `get_artifact_name_for_category`, `freeze`, `check_private_api`; then link:
   `get_link_args`, `wrap_link_actions`, `dynamic_library_*`), FeatureConfiguration,
   CcToolchainFeatures, and the `cc_common` wrappers. On the Java side: CppCompileAction,
   CcToolchainFeatures and CcToolchainVariables.
4. `xcode_config` (apple fragment, `apple_common.XcodeVersionConfig`) and
   `bazel_cc_build_info` (stamping) are both analyzed even for brotli.

**Step 3: re2.**
1. 258 `config_setting` targets, `select()` from abseil, flags (`string_flag`, `bool_flag`,
   `label_flag`) and `--features` from `.bazelrc` (`layering_check`, `parse_headers`, so
   header modules and `.d` handling).
2. Tests: `cc_test` uses `testing.ExecutionInfo`, `RunEnvironmentInfo` and
   `coverage_common.instrumented_files_info`; then the `default_test_runner` toolchain,
   TestRunnerAction, StandaloneTestStrategy, test-setup.sh and generate-xml.sh, and `test_suite`.
3. Python: the `python_repository` download, `py_builtins` (14), the py and bazel_py
   fragments, `transition` and `config.*` build settings (rules_python version flags),
   `json.decode`/`encode_indent`, `sh_binary`/`sh_library` from rules_shell, and pybind11
   `cc_binary(linkshared)` with `actions.declare_symlink` and `root_symlinks`.
4. `native.module_version` and `_copy_file` (skylib).

The Java and Apple members (about 50) can wait. They are loaded but almost all of them sit in
rule implementations that these projects never analyze. Until each one is needed it should be
`unported!` and not stubbed, so that a call shows up loudly.

## Risks

1. **The private C++ layer is the biggest piece.** `cc_internal` (33 methods) builds real
   `CppCompileAction`s and command lines from the feature engine. Matching action keys and
   command lines byte for byte means porting about 12k lines of rules/cpp, plus include
   scanning and header discovery.
2. **The load closure is large and almost identical for both projects.** brotli already needs
   repos and code from rules_java, rules_swift, rules_kotlin, rules_python and apple_support,
   because toolchain registration loads their packages. That includes repo rules that probe
   the host, such as `local_jdk` and `xcode_autoconf`.
3. **starlark-rust gaps.** Bazel puts `depset`, `struct` (with `to_json`), `select`, `Label`,
   provider callables and indexing, `set`, freezing, `json.encode_indent`, and `print` and
   `fail` formatting on top of the core language. Each must be supplied or checked against
   starlark-rust 0.14.2. Core-language uses alone number about 4.4k sites.
4. **The inventory is static.** About 500 call sites per project have an unresolved receiver,
   and 74 symbols are typed by member name only. The next step is a dynamic check: a
   `--starlark_cpu_profile` of the oracle, or a counting shim in Blaze, to confirm which native
   symbols actually execute.

## Reproduce

```
scripts/inventory/oracle-run <brotli|re2>        # under systemd-run (see AGENTS.md); runs one server for all steps
python3 scripts/inventory/java_index.py ~/.cache/blaze/src/bazel-9.3.2 ~/.cache/blaze/work/inventory/out/java-index.json
python3 scripts/inventory/extract.py files <p>; python3 scripts/inventory/extract.py uses <p>
python3 scripts/inventory/report.py docs/research/native-api.tsv
```

The `repos` step of `oracle-run` runs `mod show_repo` for every `REPOSITORY_DIRECTORY` key.
Intermediate outputs live in `~/.cache/blaze/work/inventory/out/<p>/`.
