//! Starlark rule analysis (agent S; crates/blaze_analysis/DESIGN.md): ports of
//! `analysis/starlark/{StarlarkRuleConfiguredTargetUtil, StarlarkRuleContext,
//! StarlarkAttributesCollection, StarlarkActionFactory, StarlarkToolchainContext,
//! StarlarkExecGroupCollection, StarlarkSubrule}`, `analysis/FragmentCollection`,
//! `analysis/BazelRuleAnalysisThreadContext`, and the Starlark faces of `Runfiles` and
//! `BuildConfigurationValue`.
//!
//! Entry point: [`starlark_rule_configured_target_util::build_rule`], called by
//! `ConfiguredTargetFactory` for Starlark rule classes.
//!
//! For native code running inside a rule implementation (`cc_internal`, `py_builtins`):
//! - [`thread_payload::AnalysisThreadPayload::from_or_fail`] / [`thread_payload::rule_context_from_or_fail`]
//!   give the current `RuleContext` (Java `BazelRuleAnalysisThreadContext`);
//! - [`thread_payload::register_action`] registers an action in Java's registration order;
//! - [`starlark_rule_context::starlark_rule_context_from_value`] and
//!   [`starlark_action_factory::starlark_action_factory_from_value`] unwrap `ctx` / `ctx.actions`;
//! - [`starlark_runfiles::runfiles_to_value`] / [`starlark_runfiles::runfiles_from_value`] wrap
//!   runfiles;
//! - [`fragment_collection::register_fragment_starlark_factory`] provides `ctx.fragments.<name>`.
//!
//! # Assumed APIs of other owners ("requests")
//!
//! R (`crate::rule`, `crate::providers`, `crate::configuredtargets`, `crate::test`):
//! - `RuleContext`: `get_label(&self) -> &Label`; `get_rule(&self) -> &Arc<Rule>`;
//!   `get_configuration(&self) -> &Arc<BuildConfigurationValue>`;
//!   `attributes(&self) -> &ConfiguredAttributeMapper` (C);
//!   `get_rule_prerequisites_collection(&self) -> &PrerequisitesCollection`;
//!   `get_workspace_name(&self) -> &str`; `get_features(&self) / get_disabled_features(&self) ->
//!   &IndexSet<String>`; `create_output_artifact(&mut self) -> Artifact`;
//!   `create_output_artifact_for(&mut self, out: &OutputFile) -> Artifact`;
//!   `create_output_artifact_script_for_analysis_test(&mut self) -> Artifact`;
//!   `get_output_artifacts(&self) -> Vec<Artifact>`; `get_package_directory(&self) ->
//!   PathFragment`; `get_derived_artifact / get_tree_artifact / get_shareable_artifact(&mut self,
//!   &PathFragment, &ArtifactRoot) -> Artifact`; `get_bin_directory(&self) /
//!   get_bin_or_genfiles_directory(&self) -> ArtifactRoot` (blaze_actions);
//!   `get_action_owner(&mut self) -> ActionOwner`; `get_action_owner_for_exec_group(&mut self,
//!   &str) -> Option<ActionOwner>`; `get_owner(&self) -> ArtifactOwner`;
//!   `register_action(&mut self, ActionRef)`; `rule_error(&mut self, &str)`;
//!   `rule_warning(&mut self, &str)`; `has_errors(&self) -> bool`;
//!   `throw_with_rule_error(&mut self, String) -> RuleErrorException`; `is_test_target(&self) ->
//!   bool`; `get_toolchain_context(&self) -> Option<&ResolvedToolchainContext>`;
//!   `get_toolchain_contexts(&self) -> Option<&ToolchainCollection<ResolvedToolchainContext>>`;
//!   `use_auto_exec_groups(&self) -> bool`; `get_toolchain_info(&self, &Label) ->
//!   Option<OwnedFrozenValue>`; `has_toolchain_context(&self, &str) -> bool`;
//!   `get_exec_groups(&self) -> &ExecGroupCollection` (`.get_exec_group(&str) ->
//!   &DeclaredExecGroup`); `get_execution_platform(&self, &str) -> Option<PlatformInfoRef>`;
//!   `target_platform_has_constraint(&self, &ConstraintValueInfo) -> bool`;
//!   `get_configuration_make_variable_context(&mut self) -> &ConfigurationMakeVariableContext`
//!   (`collect_make_variables(&self) -> Result<IndexMap<String, String>, ExpansionException>`,
//!   `ExpansionException::get_message(&self) -> &str`);
//!   `should_include_required_config_fragments_provider(&self) -> bool`;
//!   `get_required_config_fragments(&self, starlark_looked_up_variables:
//!   Option<IndexSet<String>>) -> Option<RequiredConfigFragmentsProvider>` (Clone);
//!   `get_default_template_variable_providers(&self) -> Vec<OwnedFrozenValue>`;
//!   `get_expander(&mut self, ConfigurationMakeVariableContext) -> Expander` with
//!   `Expander::expand(&mut self, attribute_name: &str, command: &str) -> String`;
//!   `is_legal_fragment(&self, FragmentClass) -> bool`; `get_aspect_descriptors(&self) ->
//!   &[blaze_packages::aspect::AspectDescriptor]`; `get_spawn_action_configuration(&self) -> Arc<dyn
//!   SpawnActionConfiguration>`; `get_analysis_environment(&self) -> &AnalysisEnvironment` with
//!   `&self` methods `get_event_handler() -> Arc<dyn blaze_events::EventHandler>`,
//!   `get_starlark_semantics() -> &Arc<StarlarkSemantics>`, `get_main_repo_mapping() ->
//!   RepositoryMapping`, `get_stable_workspace_status_artifact() / get_volatile_workspace_status_artifact() ->
//!   Artifact`, `get_symlink_artifact(&PathFragment, &ArtifactRoot) -> Artifact`,
//!   `get_registered_actions() -> &[ActionRef]`, `get_tree_artifacts_conflicting_with_files() /
//!   get_orphan_artifacts() -> IndexSet<Artifact>`.
//! - `rule::prerequisites_collection::PrerequisitesCollection`: `get_prerequisites(&self, &str) ->
//!   Vec<ConfiguredTarget>`; `get_executable_prerequisite(&self, &str) ->
//!   Option<Arc<FilesToRunProvider>>`; `get_prerequisite_artifact(&self, &str) ->
//!   Option<Artifact>`; `get_split_prerequisites(&self, &str) -> IndexMap<Option<String>,
//!   Vec<ConfiguredTargetAndData>>`. `rule::prerequisite_artifacts::nested_set(&PrerequisitesCollection,
//!   &str) -> NestedSet<Artifact>`. `rule::alias_provider::get_dependency_label(&ConfiguredTarget)
//!   -> Label`.
//! - `rule::rule_configured_target_builder::RuleConfiguredTargetBuilder`: `new(&RuleContext)`,
//!   `add_starlark_declared_provider(&mut self, OwnedFrozenValue)`, `set_files_to_build(&mut
//!   self, NestedSet<Artifact>)`, `add_provider<P: TransitiveInfoProvider>(&mut self, P)`,
//!   `set_runfiles_support(&mut self, Option<RunfilesSupport>, Option<Artifact>)`, `build(self,
//!   &mut RuleContext) -> Result<Option<ConfiguredTarget>, RuleConfiguredTargetBuildError>` with
//!   `enum RuleConfiguredTargetBuildError { IllegalArgument(String),
//!   Create(CreateConfiguredTargetError) }`.
//! - `rule::runfiles_support::with_executable(&mut RuleContext, Runfiles, Artifact,
//!   Option<&OwnedFrozenValue /*RunEnvironmentInfo*/>) -> Result<RunfilesSupport,
//!   CreateConfiguredTargetError>`.
//! - `rule::depsets::nested_set_to_depset<'v, E>(Heap<'v>, &NestedSet<E>, element_type: &'static
//!   str, &dyn Fn(&E, Heap<'v>) -> Value<'v>) -> Value<'v>`.
//! - `rule::artifact_roots::to_actions_artifact_root(&blaze_build_config::output_directories::ArtifactRoot)
//!   -> blaze_actions::ArtifactRoot`.
//! - `rule::configuration_make_variable_context::ConfigurationMakeVariableContext::with_overrides(
//!   Arc<PackageDeclarations>, Arc<BuildConfigurationValue>, Vec<OwnedFrozenValue>,
//!   IndexMap<String, String>)` (Java's anonymous subclass in `expandMakeVariables`).
//! - `rule::location_expander::LocationExpander::{with_exec_paths, with_runfiles_paths}(&RuleContext,
//!   IndexMap<Label, Vec<Artifact>>)`, `expand(&mut self, &str) -> Result<String, String>`.
//! - `rule::sh_toolchain::get_path_for_platform(&Arc<BuildConfigurationValue>,
//!   Option<&PlatformInfoRef>) -> PathFragment`; `rule::command_helper::{build_bash_command_constructor(&BTreeMap<String,
//!   String>, &PathFragment, &str) -> BashCommandConstructor, command_helper_script_maybe(&mut
//!   RuleContext, &str, &BashCommandConstructor, blaze_shell::OS) -> Option<Artifact>}`.
//! - `providers::default_info::DefaultInfo` (native info, StarlarkValue): `get_files() ->
//!   Option<&NestedSet<Artifact>>`, `get_stateless_runfiles() / get_data_runfiles() /
//!   get_default_runfiles() -> Option<&Runfiles>`, `get_executable() -> Option<&Artifact>`,
//!   `get_creation_location() -> &Location`.
//! - `providers::run_environment_info::RunEnvironmentInfo::should_error_on_non_executable_rule(&self)
//!   -> bool`; `providers::runfiles_provider::RunfilesProvider::{simple(Runfiles),
//!   with_data(Runfiles, Runfiles), get_default_runfiles(&self) -> &Runfiles,
//!   get_data_runfiles(&self) -> &Runfiles}` (a TransitiveInfoProvider);
//!   `providers::file_provider::FileProvider::get_files_to_build(&self) -> &NestedSet<Artifact>`;
//!   `providers::files_to_run_provider::{FilesToRunProvider (get_executable() -> Option<&Artifact>,
//!   get_files_to_run() -> &NestedSet<Artifact>), files_to_run_provider_from_value(Value) ->
//!   Option<Arc<FilesToRunProvider>>, files_to_run_provider_to_value<'v>(Heap<'v>,
//!   Arc<FilesToRunProvider>) -> Value<'v>}`;
//!   `providers::required_config_fragments_provider::RequiredConfigFragmentsProvider`.
//! - `configuredtargets::{target_to_starlark<'v>(&ConfiguredTarget, Heap<'v>) -> Value<'v>,
//!   target_from_starlark(Value) -> Option<ConfiguredTarget>}` (the Starlark `Target`).
//! - `test::instrumented_files_collector::{should_include_local_sources(&BuildConfigurationValue,
//!   &Label, bool) -> bool, should_include_local_sources_for_target(&BuildConfigurationValue,
//!   &ConfiguredTarget) -> bool}`.
//!
//! P (`crate::toolchains`, `crate::platform`): `ResolvedToolchainContext::{target_description()
//! -> &str, requested_toolchain_type_labels() -> &IndexMap<Label, _>, for_toolchain_type(&Label)
//! -> Option<OwnedFrozenValue>}`; `ToolchainCollection<T>::{get_exec_group_names() ->
//! Vec<&str>, get_toolchain_context(&str) -> Option<&T>, get_default_toolchain_context() -> &T,
//! context_map() -> &IndexMap<String, T>}`; `platform::ConstraintValueInfo` (a simple Starlark
//! value, `&ConstraintValueInfo: UnpackValue`); `platform::toolchain_type_label_from_value(Value)
//! -> Option<Label>`; `platform::get_os_from_constraints_or_host(Option<&PlatformInfoRef>) ->
//! blaze_shell::OS`.
//!
//! C (`crate::config`): `ConfiguredAttributeMapper::get(&self, &str, Type) -> Option<AttrValue>`.
//!
//! B (`blaze_packages::starlark_defs::subrule`): `SubruleView<'v> { name: String, exported:
//! Option<(Label, String)>, implementation: Value<'v>, attributes: Vec<(String /*attr*/, String
//! /*rule attr*/)>, toolchains: Vec<ToolchainTypeRequirement>, fragments: IndexSet<String>,
//! subrules: Vec<Value<'v>> }`, `subrule_view<'v>(Value<'v>) -> Option<SubruleView<'v>>`, and
//! `StarlarkSubrule::invoke` calling [`starlark_subrule::call_subrule`] (a hook the lead
//! installs).

pub mod fragment_collection;
pub mod starlark_action;
pub mod starlark_action_factory;
pub mod starlark_aspect_factory;
pub mod starlark_aspect_propagation_context;
pub mod starlark_attributes_collection;
pub mod starlark_configuration;
pub mod starlark_rule_configured_target_util;
pub mod starlark_rule_context;
pub mod starlark_runfiles;
pub mod starlark_subrule;
pub mod starlark_toolchain_context;
pub mod thread_payload;
mod util;

#[cfg(test)]
mod java_tests;

pub use starlark_rule_configured_target_util::{StarlarkRuleOutcome, build_rule};
pub use thread_payload::{AnalysisThreadPayload, RuleContextHandle, register_action};

/// Installs the analysis-phase hooks of loading-phase values: `StarlarkSubrule.call` dispatches
/// to [`starlark_subrule::call_subrule`]. The lead calls this once at startup.
pub fn install_hooks() {
    blaze_packages::starlark_defs::subrule::set_subrule_call_hook(starlark_subrule::call_subrule);
    // The provider interfaces of the builtins' parameter types (`cc_toolchain: Info`, ...).
    starlark::values::register_java_interface_matcher(crate::info::java_interface_matcher);
}
