# W4 analysis + W5 wiring: shared design

Crates: `blaze_analysis`, `blaze_rules`, `blaze_sky_analysis`, `blaze_sky_top`, `blaze_cquery`
(+ three command files in `blaze_runtime`). Goal: Blaze analyzes brotli `//...` and re2
`//:all + //python:all` like Bazel 9.3.2, so `cquery` (label_kind, jsonproto) and `aquery`
(jsonproto) match the oracle recordings (`~/.cache/blaze/oracle-records/oracle-9.3.2/<p>/norm/`),
and `blaze query|cquery|aquery` run through the real binary.

Read first: `AGENTS.md`, `PORTING.md` (Skyframe + Starlark sections), `crates/blaze_packages/DESIGN.md`
(the loading model you build on), `docs/research/04-native-api-inventory.md` (which natives run).
Java source (read-only): `~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/lib/`.

## Process

- Agents do **not** run Cargo. The lead builds and hands back the errors for your files.
- Edit only files you own (table below). Need something from a file you do not own? Call it by
  its Java name (snake_case, PORTING type mapping) as if it existed, and list the exact signature
  under "requests" in your report; the lead reconciles. Never edit another agent's file.
- New files inside your directories are fine. Module declarations (`mod.rs`) inside your own
  directories are yours; crate `lib.rs` files are the lead's (ask for a `pub mod`).
- Every ported item: `// Java: com/google/devtools/build/lib/<path>.java:<line> Class.method`.
  Unported: `blaze_util::unported!("Class.method")`, never a guessed value.
- `rustfmt --edition 2024` your files. Starlark-visible names, error texts, orders: Bazel's.
- Port the JUnit tests of what you port where feasible (`#[ignore = "needs ..."]` otherwise).

## Crates and Java packages

| crate | Java |
| --- | --- |
| `blaze_analysis` | `lib/analysis` (ConfiguredTarget*, RuleContext, providers, `analysis/starlark` ctx and actions, `analysis/platform`, `analysis/config` transitions and attribute mapping, `analysis/test`), `skyframe/ConfiguredTargetKey`, `ConfiguredTargetAndData`, toolchain context types from `skyframe/toolchains` |
| `blaze_rules` | `lib/rules/{platform,config,filegroup,test,genrule,cpp,python,objc/apple}`, `lib/rules/Alias`, `LabelBuildSettings`, `ToolchainType`, `lib/bazel/rules` analysis parts; Starlark wrappers over `blaze_cc_features` |
| `blaze_sky_analysis` | `lib/skyframe` analysis functions: `ConfiguredTargetFunction`, `DependencyResolver`, `analysis/producers/*`, `skyframe/toolchains/*` functions, `PlatformFunction`, `AspectFunction`, `IncompatibleTargetChecker` |
| `blaze_sky_top` | `SkyframeExecutor` assembly (one evaluator: loading + bzlmod + configuration + analysis functions), `SkyframeBuildView`/`BuildView.update`, `AnalysisPhaseRunner`, `TargetPatternPhase` top-level targets, test trimming at the top |
| `blaze_cquery` | `lib/query2/{PostAnalysisQueryEnvironment, cquery/*, aquery/*}`, `runtime/commands/{CqueryCommand,AqueryCommand}` bodies |

Dependency order: analysis → rules → sky_analysis → sky_top → cquery → runtime.

## Shared representations (fixed; do not redesign)

- **Configurations**: `blaze_build_config::{BuildOptions, BuildConfigurationKey,
  BuildConfigurationValue}` (all checksums already match the oracle). Configuration functions are
  `blaze_build_config::skyframe::register_configuration_functions` (client
  `ConfigurationEnvironment`); its `PlatformValueProvider` seam is implemented by agent P's
  PlatformFunction. Two `ArtifactRoot` types exist (build_config's output-directory one and
  blaze_actions'): convert with `crate::rule::artifact_roots` (agent R).
- **Keys**: `ConfiguredTargetKey` (lead, `configured_target_key.rs`, also an `ActionLookupKey`).
  Value: `ConfiguredTargetValue` (configured target + actions + `Target`).
- **Configured targets**: `ConfiguredTarget` enum (lead) over `configuredtargets::*` structs
  (agent R). `ConfiguredTargetAndData` (configured target + `Target` + configuration).
- **Providers**: `TransitiveInfoProviderMap` (lead): native-only `TransitiveInfoProvider`s by
  Rust type (FileProvider, FilesToRunProvider, RunfilesProvider, VisibilityProvider, ...) and
  `Info`s by `ProviderKey` (`blaze_packages::rule_class::ProviderKey`).
- **Infos are frozen Starlark values** (`OwnedFrozenValue`; see `info.rs`). Starlark provider
  instances are `blaze_starlark::structs::StarlarkInfo`. Native infos (DefaultInfo,
  OutputGroupInfo, RunEnvironmentInfo, PlatformInfo, ConstraintValueInfo, ToolchainInfo,
  TemplateVariableInfo, DeclaredToolchainInfo, ToolchainTypeInfo, InstrumentedFilesInfo,
  ExecutionInfo, TestEnvironmentInfo, FeatureFlagInfo, BuildSettingProvider ...) are Rust
  Starlark value types that `provide` `info::InfoProviderKey` and whose `BuiltinProviderId`/key
  equals the one `blaze_packages::starlark_defs::builtin_providers` uses (same name + value
  class), so `dep[DefaultInfo]` with the loading-phase `DefaultInfo` symbol finds them. Native
  code allocates infos with `info::freeze_native_info` and reads them with
  `info::downcast_info::<T>`. Keep Rust data (`Arc<...Data>`, `NestedSet<Artifact>`,
  `blaze_actions::analysis::runfiles::Runfiles`) inside native infos where Java keeps Java
  objects; hold Starlark values only where Java holds Starlark values (then a complex value with
  `Trace`/`Freeze`).
- **Artifacts and actions**: `blaze_actions` (`Artifact`, `ArtifactRoot`, `ArtifactFactory`,
  `ActionOwner`, `SpawnAction`/`StarlarkAction`, `FileWriteAction`, `SymlinkAction`,
  `TemplateExpansionAction`, `Args` (`MutableArgs`, `build_args`), `Runfiles`,
  `RunfilesSupport`, `SourceManifestAction`, `RepoMappingManifestAction`,
  `ActionGraphDump`). The Starlark `File` value is `blaze_actions::...::starlark_file::StarlarkFile`.
- **Depsets**: `blaze_starlark::depset::Depset` (a Starlark structure mirroring NestedSet).
  Starlark → Rust: `DepsetGen::to_nested_set` (same structure, same order). Rust → Starlark:
  `crate::rule::depsets::nested_set_to_depset` (agent R) rebuilding the same node structure,
  memoized by node identity. `to_list` orders must equal Java's.
- **Rule implementations run per configured target in a fresh `starlark::environment::Module`**
  (agent S, `starlark::starlark_rule_configured_target_util`): build `ctx`, `call_frozen` the
  rule class's implementation (`RuleClass::get_configured_target_function()`, a keep-alive
  `StarlarkCallable`), validate results, then **freeze the module** and keep the
  `FrozenHeapRef` alive inside the resulting `ConfiguredTarget` (providers are
  `OwnedFrozenValue`s on it). Values from dependencies (frozen, other heaps) are referenced with
  `OwnedFrozenValue::owned_value(module.frozen_heap())`.
- **Actions created from Starlark are finished after freezing**: `ctx.actions.run/run_shell/
  write/expand_template/...` validate arguments immediately (same errors as Java) and record a
  pending registration that holds Starlark values (e.g. `Args`). The pending list is stored in a
  module variable so freezing freezes it; after `freeze()`, agent S builds the real
  `blaze_actions` actions in registration order (`build_args` needs frozen `Args`). Actions
  native code creates directly (cc_internal's `CppCompileAction`) go into the same ordered list.
  Registration order = Java's `registerAction` order (it fixes action indices and aquery output).
- **Services**: `blaze_analysis::analysis_services::AnalysisServices` (lead-owned; ask for
  fields). The evaluator has a single client, already taken by the configuration functions
  (`ConfigurationEnvironment`), so analysis functions capture an `Arc<AnalysisServices>` in
  their registration closures: each function owner exports
  `register_*(builder: EvaluatorBuilder, services: Arc<AnalysisServices>) -> EvaluatorBuilder`.
- **Thread context**: rule implementations run with `blaze_starlark::ThreadContext` phase
  `Analysis` and a payload owned by agent S (`starlark::thread_payload::AnalysisThreadPayload`)
  giving natives (`cc_internal`, `py_builtins`, providers) access to the current `RuleContext`
  (Java `RuleContext` reached from `StarlarkRuleContext`/`StarlarkThread`).
- **Builtins / environment**: `blaze_packages`' loading environment is composed from outside
  (`blaze_packages::builtins::{BazelStarlarkEnvironment, RuleClassProviderRegistrations,
  StarlarkGlobals}`; see `blaze_sky_loading::environment`). `blaze_rules::bazel_environment`
  (lead) builds Blaze's analysis-capable environment: it starts from W3's registrations and
  **replaces** entries: `_builtins.internal.{cc_internal, cc_common, py_builtins, apple_common,
  java_common_internal_do_not_use}` and the toplevels `DefaultInfo`, `OutputGroupInfo`,
  `RunEnvironmentInfo`, `platform_common`, `testing`, `coverage_common`, `config_common` where
  analysis behaviour is needed. Each owner exports a `register_*` function; the lead wires them.
- **Native rule classes** come from `blaze_packages::native_rules` (loading). Their analysis
  factories are registered by name (`RuleClass::get_configured_target_factory()`, the Java factory
  class simple name, e.g. `"Alias"`, `"Filegroup"`, `"ConfigSetting"`, `"Platform"`,
  `"ConstraintSetting"`, `"ConstraintValue"`, `"Toolchain"`, `"ToolchainType"`, `"LabelFlag"`...)
  in `blaze_analysis::configured_rule_class_provider` (lead) by `blaze_rules::register_factories`.
- **Starlark transitions**: rules_python's `transition()` objects are W3's
  `blaze_config::transitions` values held in `blaze_packages::attribute::TransitionFactory::Starlark`.
  Agent C converts them into `blaze_build_config::starlark_transition::StarlarkDefinedConfigTransition`
  (`from_parts`, added by the lead) and applies them with
  `blaze_build_config::function_transition_util`. The exec transition is
  `function_transition_util::exec_transition_patch` with the bundled exec transition.
- **External repositories**: the `blaze_sky_loading::repos::ExternalRepos` seam; until the
  repo-rules lead lands real fetching, `OracleRepos` from `$BLAZE_ORACLE_REPOS` or
  `~/.cache/blaze/w3/<workspace basename>.repos.json` (agent Q).

## Ownership

| owner | files |
| --- | --- |
| lead | all `Cargo.toml`/`lib.rs`; `blaze_analysis/src/{analysis_services,configured_target,configured_target_key,configured_target_value,configured_target_and_data,dependency_kind,info,provider_map,configured_rule_class_provider}.rs`; `blaze_rules/src/{lib,bazel_environment,register}.rs`; `blaze_sky_analysis/src/registration.rs`; this file |
| P (platforms, toolchains) | `blaze_analysis/src/{platform,toolchains}/**`; `blaze_sky_analysis/src/{toolchains,platform}/**`; `blaze_rules/src/platform/**` (platform, constraint_setting, constraint_value, toolchain, toolchain_type factories) |
| C (configured target function, deps, transitions, select) | `blaze_analysis/src/config/**` (ConfiguredAttributeMapper, ConfigConditions, ConfigMatchingProvider, transitions, BuildSettingProvider, TransitionResolver); `blaze_sky_analysis/src/{configured_target_function.rs,dependency_resolver.rs,producers/**,incompatible_target_checker.rs,aspect_function.rs}`; `blaze_rules/src/config/**` (config_setting, alias, label_flag/label_setting) |
| R (RuleContext, providers, configured targets) | `blaze_analysis/src/{rule,providers,configuredtargets,test}/**`; `blaze_rules/src/generic/**` (filegroup, test_suite, genrule) |
| S (Starlark rule analysis) | `blaze_analysis/src/starlark/**` |
| Q (top: executor, build view, cquery/aquery, commands) | `blaze_sky_top/src/**`, `blaze_cquery/src/**` (including their lib.rs), `blaze_runtime/src/commands/{query,cquery,aquery}.rs` |
| CC (C++ natives, wave 2) | `blaze_rules/src/cpp/**` |
| PY (python, apple, fragments, build info, wave 2) | `blaze_rules/src/{python,apple,fragments,build_info}/**` |

---

# Aspects wave (2026-11-18, aspects lead)

Goal: `blaze cquery` / `aquery` / `build //src/workerd/server:workerd` on workerd match the oracle
(`docs/workerd.md`); the next blocker is aspects. Scope: a literal port of Bazel's aspect
machinery, then whatever workerd's rule sets hit next. brotli and re2 must stay identical.

## Process (differs from the first wave)

- Workers MAY run Cargo, only through `scripts/cargo-capped`, only for their own crate:
  `scripts/cargo-capped check -p <crate> --all-targets`, `scripts/cargo-capped test -p <crate> <filter>`,
  `scripts/cargo-capped clippy -p <crate> --all-targets -- -D warnings`. Builds share one target
  directory and serialize on its lock, so block in the foreground (Bash timeout 600000 ms).
- Several workers edit the same tree. Keep it compiling: write a new file completely, and add its
  `mod` line last; if your crate does not compile for more than one check cycle because of your
  files, unwire the `mod` line until it does. Errors in files you do not own: do not edit them;
  wait and re-check, and name them in your report if they persist.
- Edit only the files your brief lists. A need in another file is a "request" in your report
  (exact signature, Java reference).
- Port literally (PORTING.md): `// Java: <path>:<line> Class.method` on every item, Java error
  text and order, `unported!("Class.method")` for anything not ported, never a guess, never a
  corpus-specific shortcut. JUnit tests of what you port, with real bodies and Java's expected
  strings; `#[ignore = "needs <what>"]` only for missing infrastructure.
- `rustfmt --edition 2024` on every file you touch. No commits. No files outside the repo
  except scratch under your scratchpad. Do not edit `scripts/oracle`, `scripts/gate`, `corpus/`,
  `docs/state`, `docs/reviews`, AGENTS.md, PORTING.md.

## Model

| Java | Rust |
| --- | --- |
| `packages/{Aspect, AspectClass, NativeAspectClass, StarlarkAspectClass, AspectDescriptor, AspectParameters, AspectDefinition, AspectPropagationEdgesSupplier, AspectPropagationPredicate}` | `blaze_packages::aspect::*` (new module; the Starlark value stays `starlark_defs::aspect::StarlarkDefinedAspect`, which gains `get_definition(&AspectParameters)`, `get_aspect_class()`, top-level parameter extraction) |
| `AspectsList.getAspects(Rule)`, `Attribute.getAspects(Rule)`, `Rule.getAspectLabelsSuperset` | methods on the existing types |
| `skyframe/AspectKeyCreator` (`AspectKey`, `TopLevelAspectsKey`), `analysis/AspectValue`, `ConfiguredAspect`, `AspectCollection`, `AspectResolutionHelpers`, `AspectContext`, `AspectAwareAttributeMapper`, `AspectBaseTargetResolvedToolchainContext`, `MergedConfiguredTarget`, `ConfiguredTargetFactory.createAspect`, `RuleContext` aspect parts | `blaze_analysis` (one file per Java class, snake_case of the class name; keys next to `configured_target_key.rs`) |
| `analysis/starlark/{StarlarkAspectFactory (skyframe/), StarlarkAspectPropagationContext}`, the aspect half of `StarlarkRuleContext` / `StarlarkAttributesCollection` (`ctx.rule`, `ctx.aspect_ids`, aspect `ctx.attr`) | `blaze_analysis::starlark` |
| `skyframe/{AspectFunction, ToplevelStarlarkAspectFunction, LoadAspectsFunction}` (`BuildTopLevelAspectsDetailsFunction` does not exist at the pin: `LoadAspectsFunction` replaced it), the aspect parts of `DependencyResolver`, `analysis/producers/{DependencyMapProducer, DependencyProducer, PrerequisitesProducer, ConfiguredAspectProducer}`, `DependencyResolutionHelpers` | `blaze_sky_analysis` |
| `BuildView` / `SkyframeBuildView` top-level aspects, `AspectCompletor`, cycle and error reporting for aspect keys, cquery/aquery aspect values | `blaze_sky_top`, `blaze_cquery` |

Fixed points:

- An `Aspect` is `Clone + Send + Sync + Eq + Hash`; equality is Java's (descriptor equality and
  the *identity* of the definition; definitions are cached per aspect and parameters exactly as
  Java's `definitionCache`, so identity is stable).
- Starlark functions held by definitions (`attr_aspects` / `toolchains_aspects` functions,
  propagation predicate, implementation) are `blaze_packages::rule_class::StarlarkCallable`
  keep-alive handles to frozen values, as rule implementations already are.
- `AspectKey` implements `blaze_skyframe::Key` (`FUNCTION = "ASPECT"`, value `AspectValue`) and
  `blaze_actions::ActionLookupKey`; its `Debug` is Java's `toString`.
- `BaseTargetPrerequisitesSupplier` (Java looks the base target's dependencies up in the graph
  without adding edges) is ported as it is: `blaze_analysis::base_target_prerequisites_supplier`
  over `Evaluator::get_existing_value`, reached through
  `AnalysisServices::get_base_target_prerequisites_supplier()`. It must NOT become an ordinary
  Skyframe request: the edges of an `ASPECT` node are what cquery walks, so extra edges would
  change cquery's output order. (Corrected 2026-11-18; an earlier version of this section said
  the opposite.)
- Java's `SkyKeyComputeState` memoization is dropped as everywhere else in these crates (the
  evaluator blocks instead of restarting); keep the order of requests.

## Ownership (aspects wave)

| worker | files |
| --- | --- |
| W1 packages model | `blaze_packages/src/aspect/**`, `starlark_defs/aspect.rs`, `rule_class/aspects_list.rs`, aspect methods of `attribute/attribute.rs` and `target/rule.rs` |
| W2 harness | `blaze_sky_top/src/testing/**`, `blaze_sky_top/src/tests/**` |
| W3 analysis core | `blaze_analysis/src/{aspect_key_creator, aspect_value, configured_aspect, configured_aspect_factory, aspect_collection, aspect_resolution_helpers, inconsistent_aspect_order_exception, aspect_creation_exception, aspect_aware_attribute_mapper}.rs`, `toolchains/aspect_base_target_resolved_toolchain_context.rs`, `configuredtargets/**` (`MergedConfiguredTarget`), `configured_target.rs`, `configured_target_and_data.rs`, `dependency_kind.rs`, `provider_map.rs` |
| W4 rule context and Starlark | `blaze_analysis/src/aspect_context.rs`, `blaze_analysis/src/rule/**` (`RuleContext` aspect parts, `ConfiguredTargetFactory.createAspect`), `blaze_analysis/src/starlark/**` (`StarlarkAspectFactory`, `StarlarkAspectPropagationContext`, `ctx.rule`, aspect `ctx`), the aspect `unported!` sites in `blaze_rules/src/cpp/cc_starlark_internal.rs` |
| W5 Skyframe | `blaze_sky_analysis/src/**` |
| W6 top level, cquery, aquery (starts after W3) | `blaze_sky_top/src/**` except W2's directories, `blaze_cquery/src/**` |
| lead | every `Cargo.toml`, `lib.rs`, `mod.rs` conflicts, `analysis_services.rs`, this file, `docs/workerd.md` |

`lib.rs` / `mod.rs`: each worker adds its own `pub mod` lines (one line per file, alphabetical),
re-reading the file right before the edit; nothing else in those files.

## Contracts between workers

Names and parameter orders are Java's (PORTING "Names", "Types"); a function that has a Rust
neighbour mirrors the neighbour's Rust types (for example `ConfiguredTargetFactory::create_aspect`
takes what `create_configured_target` takes for the same Java parameters). Workers draft against
these and reconcile with the real files when their own code is written; a signature that is
wrong against Java is reported to the lead, not patched around.

- W1 → all: `blaze_packages::aspect::{Aspect, AspectClass, StarlarkAspectClass, NativeAspectClass,
  AspectDescriptor, AspectParameters, AspectDefinition, AspectPropagationEdgesSupplier,
  AspectPropagationPredicate}`; `Attribute::get_aspects(&Rule) -> Vec<Aspect>`;
  `StarlarkDefinedAspect` accessors in `starlark_defs::aspect`.
- W3 → W4, W5, W6:
  - `aspect_key_creator::{AspectKey, TopLevelAspectsKey, AspectBaseKey?, ASPECT, TOP_LEVEL_ASPECTS}`,
    `AspectKeyCreator::create_aspect_key(descriptor, base_keys, base_configured_target_key)`,
    `create_top_level_aspects_key(...)`, `AspectKey::{get_base_keys, get_base_configured_target_key,
    get_aspect_descriptor, get_aspect_class, get_parameters, get_label, get_configuration_key,
    get_description, with_label, pretty_print}`; `impl Key for AspectKey { type Value = AspectValue }`.
  - `aspect_value::AspectValue::{create, create_for_alias, get_aspect, get_providers, get_actions,
    get_transitive_packages, get_key_for_transitive_package_tracking, is_for_alias_target}`.
  - `configured_aspect::{ConfiguredAspect (providers + actions; `NonApplicableAspect`; `for_alias`),
    ConfiguredAspectBuilder}`; `configured_aspect_factory::ConfiguredAspectFactory` (trait, Java's
    `create(label, ct_and_data, rule_context, parameters, tools_repository)`).
  - `aspect_collection::{AspectCollection, AspectDeps, AspectCycleOnPathException, build_aspect_key}`,
    `aspect_resolution_helpers::{compute_propagating_aspects, compute_aspect_collection,
    compute_aspect_collection_no_aspects_filtering, compute_attribute_aspects,
    compute_toolchains_aspects}`, `InconsistentAspectOrderException`, `AspectCreationException`.
  - `MergedConfiguredTarget::of(base, &[ConfiguredAspect]) -> Result<ConfiguredTarget, MergingException>`.
  - `AspectAwareAttributeMapper`, `AspectBaseTargetResolvedToolchainContext::load`.
- W4 → W3, W5: `starlark::starlark_aspect_propagation_context::StarlarkAspectPropagationContext::
  {create_for_propagation_predicate, create_for_propagation_edges}` (in the shape W1's suppliers
  take); `starlark::starlark_aspect_factory::StarlarkAspectFactory::new(aspect)`;
  `rule::configured_target_factory::ConfiguredTargetFactory::create_aspect(...)` (Java parameter
  order); `aspect_context::AspectContext`.
- W5 → W6: `ASPECT` nodes with `AspectValue`s in the graph; `TOP_LEVEL_ASPECTS` and
  `LOAD_ASPECTS` functions (`register` in each file; `AspectCreationException` lives in
  `blaze_sky_analysis::producers::errors`, next to `Cause`).
