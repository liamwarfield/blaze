//! `ctx.actions`: port of `analysis/starlark/StarlarkActionFactory.java`
//! (`starlarkbuildapi/StarlarkActionFactoryApi.java`).
//!
//! Arguments are validated when the method is called, with Java's errors. The action itself is
//! registered in the rule's ordered list of pending actions (see `thread_payload`): actions that
//! hold no Starlark values are built at once; `run`/`run_shell` (whose `Args` must be frozen
//! before their command lines can be built), `write` with an `Args` content and
//! `expand_template` with `computed_substitutions` are built by
//! [`finish_pending_actions`] after the rule's module is frozen, in registration order.
//!
//! Java reads an `Args` (`Args.build`, `getParamFileInfo`, ...) and a `TemplateDict` (`getAll`)
//! when the action is created, so later calls on the same object do not change the action.
//! Here a copy of the object's state is taken at that point (`snapshot_args`,
//! `snapshot_template_dict`) and the copy is what is frozen and built.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use blaze_packages::bzl_context::BazelModuleContext;

use allocative::Allocative;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::command_line;
use blaze_actions::actions::path_mapper::FileRoot;
use blaze_actions::analysis::actions::abstract_file_write_action;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
use blaze_actions::analysis::actions::spawn_action::{ResourceSet, SpawnActionBuilder};
use blaze_actions::analysis::actions::starlark_action::{
    set_unused_inputs_list, starlark_action_builder,
};
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::starlark::args::{build_args, is_args, new_args, snapshot_args};
use blaze_actions::analysis::starlark::starlark_file::{
    StarlarkFile, StarlarkFileRoot, artifact_from_value,
};
use blaze_actions::analysis::starlark::template_dict::{
    get_all, is_template_dict, new_template_dict, snapshot_template_dict,
};
use blaze_actions::analysis::starlark::unresolved_symlink_action::{
    SymlinkTargetType, UnresolvedSymlinkAction,
};
use blaze_actions::{ActionRef, Artifact, ArtifactRoot};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_packages::rule_class::declared_exec_group::DEFAULT_EXEC_GROUP_NAME;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_shell::util::os::OS;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::none::NoneOr;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue,
    Trace, Tracer, Value, starlark_value,
};

use super::starlark_configuration::allow_unresolved_symlinks;
use super::starlark_toolchain_context::is_valid_group_name;
use super::thread_payload::{
    AnalysisThreadPayload, CommandLinePart, FrozenAnalysisKeepAlive, PendingAction,
    RuleContextHandle, keep_alive_push,
};
use super::util::{
    check_bool_param, depset_cast_artifacts, depset_get_set_artifacts, depset_to_list,
    dict_cast_string_object, dict_cast_string_string, errorf,
    fail_if_called_outside_default_allowlist, get_filtered_execution_info, is_sequence,
    param_type_error, sequence_cast_artifacts, sequence_cast_strings, sequence_elements,
};
use crate::platform::PlatformInfo;
use crate::providers::file_provider::FilesToRunProvider;
use crate::rule::RuleContext;

/// What the action factory needs from its owner (Java
/// `StarlarkActionFactory.StarlarkActionContext`, implemented by `StarlarkRuleContext` and the
/// subrule context).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1292 StarlarkActionFactory.StarlarkActionContext
#[derive(Clone)]
pub struct StarlarkActionContext {
    pub(crate) handle: RuleContextHandle,
    /// State shared with the `ctx` value (subrule locking).
    pub(crate) shared: Arc<SharedRuleContextState>,
    /// Java `StarlarkRuleContext.ruleLabelCanonicalName`.
    pub(crate) rule_label_canonical_name: String,
    /// Java `StarlarkRuleContext.isForAspect()`.
    pub(crate) is_for_aspect: bool,
    /// Java `StarlarkAttributesCollection.getExecutableRunfilesMap()`.
    pub(crate) executable_runfiles_map: Arc<IndexMap<Artifact, Arc<FilesToRunProvider>>>,
    /// Set when this is a subrule context (Java `StarlarkSubrule.SubruleContext`).
    pub(crate) subrule: Option<Arc<SubruleActionState>>,
}

/// State shared by `ctx`, `ctx.outputs`, `ctx.actions` and subrule contexts.
#[derive(Debug, Default)]
pub struct SharedRuleContextState {
    /// Java `StarlarkRuleContext.lockedForSubruleEvaluation`: 0 when not locked, else the id of
    /// the subrule context that locked it.
    pub(crate) locked_by: std::sync::atomic::AtomicU64,
    /// Ids for subrule contexts.
    pub(crate) next_subrule_id: std::sync::atomic::AtomicU64,
}

/// The parts of Java's `SubruleContext` the action factory uses.
#[derive(Debug)]
pub struct SubruleActionState {
    /// Identity of the subrule context (Java object identity).
    pub(crate) id: u64,
    /// Java `subrule.getName()`.
    pub(crate) name: String,
    /// Java `requestedToolchains`.
    pub(crate) requested_toolchains: indexmap::IndexSet<Label>,
    /// Java `runfilesFromDeps`.
    pub(crate) runfiles_from_deps: Vec<Arc<FilesToRunProvider>>,
    /// False once Java's `nullify()` ran.
    pub(crate) alive: std::sync::atomic::AtomicBool,
}

impl StarlarkActionContext {
    /// Java `isImmutable()` of the rule context (locked by a subrule) or of the subrule context
    /// (`starlarkRuleContext.getLockedForSubrule() != this`).
    pub(crate) fn is_immutable(&self) -> bool {
        let locked_by = self
            .shared
            .locked_by
            .load(std::sync::atomic::Ordering::Relaxed);
        match &self.subrule {
            None => locked_by != 0,
            Some(s) => !s.alive.load(std::sync::atomic::Ordering::Relaxed) || locked_by != s.id,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:494 StarlarkRuleContext.checkMutable
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:420 SubruleContext.checkMutable
    pub(crate) fn check_mutable(&self, attr_name: &str) -> starlark::Result<()> {
        if self.is_immutable() {
            if self.subrule.is_some() {
                return Err(errorf(format!(
                    "cannot access field or method '{attr_name}' of subrule context outside of \
                     its own implementation function"
                )));
            }
            return Err(closed_error(attr_name, &self.rule_label_canonical_name));
        }
        Ok(())
    }

    /// Java `newFileRoot()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:486 StarlarkRuleContext.newFileRoot
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:415 SubruleContext.newFileRoot
    pub(crate) fn new_file_root(&self) -> ArtifactRoot {
        let rc = self.handle.borrow();
        if self.is_for_aspect || self.subrule.is_some() {
            rc.get_bin_directory()
        } else {
            rc.get_bin_or_genfiles_directory()
        }
    }

    /// Java `getExecutableRunfiles(executable, what)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:981 StarlarkRuleContext.getExecutableRunfiles
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:436 SubruleContext.getExecutableRunfiles
    pub(crate) fn get_executable_runfiles(
        &self,
        executable: &Artifact,
        what: &str,
    ) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        if let Some(subrule) = &self.subrule {
            if subrule
                .runfiles_from_deps
                .iter()
                .any(|dep| dep.get_executable() == Some(executable))
            {
                // TODO: b/293304174 - maybe return the matched FilesToRunProvider instead of
                // failing?
                return Err(errorf(format!(
                    "for '{what}', expected FilesToRunProvider, got File"
                )));
            }
            // executable attributes of a subrule are passed to the implementation as
            // FilesToRunProvider so this should never happen unless this comes from somewhere
            // else, in which case, we can't resolve it anyway
            return Ok(None);
        }
        Ok(self.executable_runfiles_map.get(executable).cloned())
    }

    /// Java `areRunfilesFromDeps(executable)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:990 StarlarkRuleContext.areRunfilesFromDeps
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:450 SubruleContext.areRunfilesFromDeps
    pub(crate) fn are_runfiles_from_deps(&self, executable: &Arc<FilesToRunProvider>) -> bool {
        if let Some(subrule) = &self.subrule {
            return subrule
                .runfiles_from_deps
                .iter()
                .any(|p| same_files_to_run_provider(p, executable));
        }
        self.executable_runfiles_map
            .values()
            .any(|p| same_files_to_run_provider(p, executable))
    }

    /// Java `maybeOverrideExecGroup(execGroupUnchecked)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:465 SubruleContext.maybeOverrideExecGroup
    pub(crate) fn maybe_override_exec_group<'a>(
        &self,
        exec_group_unchecked: Option<&'a str>,
    ) -> starlark::Result<Option<&'a str>> {
        if self.subrule.is_some() && exec_group_unchecked.is_some() {
            return Err(errorf("'exec_group' may not be specified in subrules"));
        }
        // TODO: b/293304174 - return the correct exec group
        Ok(exec_group_unchecked)
    }

    /// Java `maybeOverrideToolchain(toolchainUnchecked)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:474 SubruleContext.maybeOverrideToolchain
    pub(crate) fn maybe_override_toolchain<'v>(
        &self,
        toolchain_unchecked: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Option<Value<'v>>> {
        let Some(subrule) = &self.subrule else {
            return Ok(toolchain_unchecked);
        };
        if toolchain_unchecked.is_some() {
            return Err(errorf("'toolchain' may not be specified in subrules"));
        }
        Ok(Some(if subrule.requested_toolchains.is_empty() {
            Value::new_none()
        } else {
            blaze_util::check_argument!(subrule.requested_toolchains.len() == 1);
            heap.alloc(LabelValue(
                subrule.requested_toolchains.first().unwrap().clone(),
            ))
        }))
    }

    /// Java `getStarlarkSemantics()`.
    pub(crate) fn get_starlark_semantics(&self) -> Arc<StarlarkSemantics> {
        self.handle
            .borrow()
            .get_analysis_environment()
            .get_starlark_semantics()
            .clone()
    }

    /// Java `context.repr(printer)` of the rule context.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:587 StarlarkRuleContext.repr
    pub(crate) fn repr(&self) -> String {
        match &self.subrule {
            // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:494 SubruleContext.repr
            Some(s) => format!(
                "<{} context for {}>",
                s.name,
                self.handle.borrow().get_label()
            ),
            None => context_repr(self.is_for_aspect, &self.rule_label_canonical_name),
        }
    }
}

/// Java `StarlarkRuleContext.repr`.
pub(crate) fn context_repr(is_for_aspect: bool, rule_label_canonical_name: &str) -> String {
    if is_for_aspect {
        format!("<aspect context for {rule_label_canonical_name}>")
    } else {
        format!("<rule context for {rule_label_canonical_name}>")
    }
}

/// The `checkMutable` error of a closed (or locked) rule context.
pub(crate) fn closed_error(attr_name: &str, rule_label_canonical_name: &str) -> starlark::Error {
    errorf(format!(
        "cannot access field or method '{attr_name}' of rule context for \
         '{rule_label_canonical_name}' outside of its own rule implementation function"
    ))
}

// -------------------------------------------------------------------------------------------
// The Starlark value

/// The value of `ctx.actions` (type `actions`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:113 StarlarkActionFactory
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkActionFactory {
    #[allocative(skip)]
    context: StarlarkActionContext,
    /// Counter for actions.run_shell helper scripts. Every script must have a unique name.
    #[allocative(skip)]
    run_shell_output_counter: Cell<i32>,
}

/// `ctx.actions` after the rule implementation returned (Java: the factory of a closed
/// `StarlarkRuleContext`, whose `checkMutable` always fails).
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct ClosedStarlarkActionFactory {
    rule_label_canonical_name: String,
    is_for_aspect: bool,
}

impl fmt::Debug for StarlarkActionFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StarlarkActionFactory")
    }
}

impl fmt::Display for StarlarkActionFactory {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1286 StarlarkActionFactory.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "actions for{}", self.context.repr())
    }
}

impl fmt::Debug for ClosedStarlarkActionFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClosedStarlarkActionFactory")
    }
}

impl fmt::Display for ClosedStarlarkActionFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "actions for{}",
            context_repr(self.is_for_aspect, &self.rule_label_canonical_name)
        )
    }
}

// SAFETY: holds no Starlark values.
unsafe impl<'v> Trace<'v> for StarlarkActionFactory {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl Freeze for StarlarkActionFactory {
    type Frozen = ClosedStarlarkActionFactory;

    // Java: StarlarkRuleContext.close() nullifies the rule context the factory uses.
    fn freeze(self, _freezer: &Freezer) -> FreezeResult<ClosedStarlarkActionFactory> {
        Ok(ClosedStarlarkActionFactory {
            rule_label_canonical_name: self.context.rule_label_canonical_name,
            is_for_aspect: self.context.is_for_aspect,
        })
    }
}

impl<'v> AllocValue<'v> for StarlarkActionFactory {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "actions")]
impl<'v> StarlarkValue<'v> for StarlarkActionFactory {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("actions", actions_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

#[starlark_value(type = "actions")]
impl<'v> StarlarkValue<'v> for ClosedStarlarkActionFactory {
    type Canonical = StarlarkActionFactory;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("actions", actions_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

impl StarlarkActionFactory {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:121 StarlarkActionFactory.<init>
    pub(crate) fn new(context: StarlarkActionContext) -> StarlarkActionFactory {
        StarlarkActionFactory {
            context,
            run_shell_output_counter: Cell::new(0),
        }
    }

    /// The factory's context (Java `context`).
    pub fn context(&self) -> &StarlarkActionContext {
        &self.context
    }

    /// Java `getRuleContext()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:560 StarlarkActionFactory.getRuleContext
    pub fn get_rule_context(&self) -> RuleContextHandle {
        self.context.handle
    }
}

/// The live factory behind `this` after `context.checkMutable(attr_name)`.
fn factory<'v>(this: Value<'v>, attr_name: &str) -> starlark::Result<&'v StarlarkActionFactory> {
    if let Some(f) = this.downcast_ref::<StarlarkActionFactory>() {
        f.context.check_mutable(attr_name)?;
        return Ok(f);
    }
    if let Some(c) = this.downcast_ref::<ClosedStarlarkActionFactory>() {
        return Err(closed_error(attr_name, &c.rule_label_canonical_name));
    }
    Err(errorf(format!(
        "internal error: expected actions, got {}",
        this.get_type()
    )))
}

/// The live `ctx.actions` value of `v`, for natives that receive it (Java `(StarlarkActionFactory)
/// actions`), e.g. `cc_internal.actions2ctx_cheat`.
pub fn starlark_action_factory_from_value<'v>(v: Value<'v>) -> Option<&'v StarlarkActionFactory> {
    v.downcast_ref::<StarlarkActionFactory>()
}

// -------------------------------------------------------------------------------------------
// Helpers

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:131 StarlarkActionFactory.checkToolchainParameterIsSet
fn check_toolchain_parameter_is_set(
    rule_context: &RuleContext,
    toolchain_unchecked: Option<Value>,
) -> starlark::Result<()> {
    let contexts_unusable = match rule_context.get_toolchain_contexts() {
        None => true,
        Some(c) => c.context_map().len() > 1,
    };
    if contexts_unusable && toolchain_unchecked.is_none() {
        return Err(errorf(
            "Couldn't identify if tools are from implicit dependencies or a toolchain. Please set \
             the toolchain parameter. If you're not using a toolchain, set it to 'None'.",
        ));
    }
    Ok(())
}

/// The declared path of a new artifact: `filename` under the package directory, or next to
/// `sibling`.
fn declared_path(
    rule_context: &RuleContext,
    filename: &str,
    sibling: Option<&Artifact>,
    sibling_repository_layout: bool,
) -> PathFragment {
    match sibling {
        None => rule_context
            .get_package_directory()
            .get_relative(&PathFragment::create(filename)),
        Some(sibling) => {
            let original = sibling.get_output_dir_relative_path(sibling_repository_layout);
            original
                .replace_name(filename)
                .expect("NullPointerException: PathFragment.replaceName")
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:568 StarlarkActionFactory.verifyExecGroupExists
fn verify_exec_group_exists(exec_group: &str, ctx: &RuleContext) -> starlark::Result<()> {
    if !ctx.has_toolchain_context(exec_group) {
        return Err(errorf(format!(
            "Action declared for non-existent exec group '{exec_group}'."
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:575 StarlarkActionFactory.verifyAutomaticExecGroupExists
fn verify_automatic_exec_group_exists(
    exec_group: &str,
    rule_context: &RuleContext,
) -> starlark::Result<()> {
    if !rule_context.has_toolchain_context(exec_group) {
        return Err(errorf(format!(
            "Action declared for non-existent toolchain '{exec_group}'."
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:582 StarlarkActionFactory.checkValidGroupName
fn check_valid_group_name(exec_group: &str) -> starlark::Result<()> {
    if !is_valid_group_name(exec_group) {
        return Err(errorf(format!(
            "Invalid name for exec group '{exec_group}'."
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:588 StarlarkActionFactory.getExecutionPlatform
fn get_execution_platform(
    exec_group_unchecked: Option<&str>,
    ctx: &RuleContext,
) -> starlark::Result<Option<Arc<PlatformInfo>>> {
    match exec_group_unchecked {
        None => Ok(ctx.get_execution_platform_for(DEFAULT_EXEC_GROUP_NAME)),
        Some(exec_group) => {
            verify_exec_group_exists(exec_group, ctx)?;
            check_valid_group_name(exec_group)?;
            Ok(ctx.get_execution_platform_for(exec_group))
        }
    }
}

/// Java `getMnemonic(mnemonicUnchecked, defaultMnemonic)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1124 StarlarkActionFactory.getMnemonic
fn get_mnemonic(
    rule_context: &RuleContext,
    mnemonic_unchecked: Option<&str>,
    default_mnemonic: &str,
) -> String {
    let mnemonic = mnemonic_unchecked.unwrap_or(default_mnemonic).to_owned();
    if rule_context
        .get_configuration()
        .get_reserved_action_mnemonics()
        .iter()
        .any(|m| m == &mnemonic)
    {
        return mangle_mnemonic(&mnemonic);
    }
    mnemonic
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1133 StarlarkActionFactory.mangleMnemonic
fn mangle_mnemonic(mnemonic: &str) -> String {
    format!("{mnemonic}FromStarlark")
}

/// `SpawnAction.Builder.setMnemonic`'s precondition, as the EvalException Java reports.
// Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:1258 SpawnAction.Builder.setMnemonic
fn set_mnemonic(builder: &mut SpawnActionBuilder, mnemonic: &str) -> starlark::Result<()> {
    if !blaze_actions::analysis::actions::spawn_action::is_valid_mnemonic(mnemonic) {
        return Err(errorf(format!(
            "mnemonic must only contain letters and/or digits, and have non-zero length, was: \
             \"{mnemonic}\""
        )));
    }
    builder.set_mnemonic(mnemonic);
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:520 StarlarkActionFactory.validateActionCreation
fn validate_action_creation(rule_context: &RuleContext) -> starlark::Result<()> {
    // We check if the rule is a dependency resolution rule but allow aspects attached to them.
    // The idea is that dependency resolution rules should not depend on anything other than
    // dependency resolution rules but since there is no such thing as "dependency resolution
    // aspect", there is no risk of that with aspects.
    if rule_context.get_aspect_descriptors().is_empty()
        && rule_context
            .get_rule()
            .get_rule_class_object()
            .is_dependency_resolution_rule()
    {
        return Err(errorf(
            "rules that can be required for materializers shouldn't have actions",
        ));
    }
    if rule_context.get_rule().is_analysis_test() {
        return Err(errorf(
            "implementation function of a rule with analysis_test=true may not register actions. \
             Analysis test rules may only return success/failure information via \
             AnalysisTestResultInfo.",
        ));
    }
    Ok(())
}

/// Java `registerAction(action)`: validates, then appends to the rule's ordered action list.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:545 StarlarkActionFactory.registerAction
fn register_action(
    f: &StarlarkActionFactory,
    action: PendingAction,
    eval: &Evaluator,
) -> starlark::Result<()> {
    validate_action_creation(&f.context.handle.borrow())?;
    AnalysisThreadPayload::from_or_fail(eval, "registerAction")?.push_pending(action);
    Ok(())
}

/// Java `StarlarkActionFactory.buildCommandLine(builder, argumentsList, repoMappingSupplier)`:
/// the command line parts, with a copy of each `Args` as it is now kept alive for the build
/// after freezing.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:695 StarlarkActionFactory.buildCommandLine
fn build_command_line<'v>(
    arguments_list: &[Value<'v>],
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Vec<CommandLinePart>> {
    let mut parts = Vec::new();
    let mut string_args: Option<Vec<String>> = None;
    for value in arguments_list {
        if let Some(s) = value.unpack_str() {
            string_args.get_or_insert_with(Vec::new).push(s.to_owned());
        } else if is_args(*value) {
            if let Some(strings) = string_args.take() {
                parts.push(CommandLinePart::Strings(strings));
            }
            // Java: ParamFileInfo paramFileInfo = args.getParamFileInfo();
            //       builder.addCommandLine(args.build(repoMappingSupplier), paramFileInfo);
            // `args.build` copies the arguments added so far
            // (com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:882
            // StarlarkCustomCommandLine.Builder.build: `arguments.toArray()`), and the param file
            // settings are read here, so `args.add(...)`, `use_param_file` or
            // `set_param_file_format` called afterwards do not reach this action.
            // PORT: the command line can only be built from frozen values, so the state of the
            // Args object at this point is copied, and the copy is built after freezing.
            let snapshot = snapshot_args(*value, eval.heap());
            parts.push(CommandLinePart::Args(keep_alive_push(eval, snapshot)?));
        } else {
            return Err(errorf(format!(
                "expected list of strings or ctx.actions.args() for arguments instead of {}",
                value.get_type()
            )));
        }
    }
    if let Some(strings) = string_args {
        parts.push(CommandLinePart::Strings(strings));
    }
    Ok(parts)
}

/// Java `determineExecGroup(ruleContext, execGroupUnchecked, toolchainUnchecked)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:844 StarlarkActionFactory.determineExecGroup
pub fn determine_exec_group(
    rule_context: &RuleContext,
    exec_group_unchecked: Option<&str>,
    toolchain_unchecked: Option<Value>,
    eval: &Evaluator,
) -> starlark::Result<String> {
    let mut toolchain_label: Option<Label> = None;
    if let Some(t) = toolchain_unchecked {
        if let Some(label) = t.downcast_ref::<LabelValue>() {
            toolchain_label = Some(label.label().clone());
        } else if let Some(s) = t.unpack_str() {
            let converter = LabelConverter::for_bzl_evaluating_thread(eval)
                .map_err(|e| errorf(e.get_message()))?;
            toolchain_label = Some(converter.convert(s).map_err(|e| errorf(e.get_message()))?);
        }
    }

    if let Some(exec_group) = exec_group_unchecked {
        verify_exec_group_exists(exec_group, rule_context)?;
        check_valid_group_name(exec_group)?;

        // If toolchain and exec_groups are both defined, verify they are compatible.
        if rule_context.use_auto_exec_groups() {
            if let Some(toolchain_label) = &toolchain_label {
                let compatible = rule_context
                    .get_exec_groups()
                    .get_exec_group(exec_group)
                    .expect("NullPointerException: exec group")
                    .toolchain_types()
                    .iter()
                    .any(|t| &t.toolchain_type == toolchain_label);
                if !compatible {
                    return Err(errorf(
                        "`toolchain` and `exec_group` parameters inside actions.{run, run_shell} \
                         are not compatible; use one of them or define `toolchain` which is \
                         compatible with the exec_group (already exists inside the `exec_group`)",
                    ));
                }
            }
        }
        return Ok(exec_group.to_owned());
    } else if rule_context.use_auto_exec_groups() {
        if let Some(toolchain_label) = &toolchain_label {
            verify_automatic_exec_group_exists(&toolchain_label.to_string(), rule_context)?;
            return Ok(toolchain_label.to_string());
        }
    }
    Ok(DEFAULT_EXEC_GROUP_NAME.to_owned())
}

const VALID_RESOURCES: [&str; 3] = ["cpu", "memory", "local_test"];

/// `StarlarkActionResourceSetBuilder.getNumericOrDefault`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:953 StarlarkActionResourceSetBuilder.getNumericOrDefault
fn get_numeric_or_default(
    resource_set_map: &IndexMap<String, Value>,
    key: &str,
    default_value: f64,
) -> starlark::Result<f64> {
    let Some(value) = resource_set_map.get(key) else {
        return Ok(default_value);
    };
    if let Some(i) = value.unpack_i32() {
        return Ok(i as f64);
    }
    if value.get_type() == "int" {
        // Java: StarlarkInt.toDouble of a big int.
        return Ok(value.to_str().parse::<f64>().unwrap_or(f64::INFINITY));
    }
    if value.get_type() == "float" {
        return Ok(value.to_str().parse::<f64>().expect("float"));
    }
    Err(errorf(format!(
        "Illegal resource value type for key {key}: got {}, want int or float",
        value.get_type()
    )))
}

/// `parseResourceSetFromDict(resourceSetUnchecked)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1036 StarlarkActionFactory.parseResourceSetFromDict
fn parse_resource_set_from_dict(resource_set_unchecked: Value) -> starlark::Result<ResourceSet> {
    let resource_set_map_raw = dict_cast_string_object(resource_set_unchecked, "resource_set")?;
    let illegal: Vec<&str> = resource_set_map_raw
        .keys()
        .map(|k| k.as_str())
        .filter(|k| !VALID_RESOURCES.contains(k))
        .collect();
    if !illegal.is_empty() {
        return Err(errorf(format!(
            "Illegal resource keys: ({})",
            illegal.join(",")
        )));
    }
    let default = ResourceSet::default_resource_set();
    let memory_mb = get_numeric_or_default(&resource_set_map_raw, "memory", default.memory_mb)?;
    let cpu_usage = get_numeric_or_default(&resource_set_map_raw, "cpu", default.cpu_usage)?;
    let local_test_count = get_numeric_or_default(
        &resource_set_map_raw,
        "local_test",
        default.local_test_count as f64,
    )? as i32;
    // Optimize for low retained memory usage since this resource set is retained by the action.
    // The loss of precision for memory and CPU usage due to the cast to float is negligible for
    // the purpose of resource scheduling.
    if local_test_count == default.local_test_count {
        // Java: StarlarkActionResourceSet.create((float) memoryMb, (float) cpuUsage)
        return Ok(ResourceSet {
            memory_mb: (memory_mb as f32) as f64,
            cpu_usage: (cpu_usage as f32) as f64,
            extra_resource_usage: IndexMap::new(),
            local_test_count: 0,
        });
    }
    Ok(ResourceSet {
        memory_mb,
        cpu_usage,
        extra_resource_usage: IndexMap::new(),
        local_test_count,
    })
}

/// `print()` in a `resource_set` callback: Java's thread keeps `StarlarkThread`'s default print
/// handler, `System.err.println(thread.getCallerLocation() + ": " + msg)` (the server's stderr).
// Java: net/starlark/java/eval/StarlarkThread.java:366 StarlarkThread.defaultPrintHandler
struct ResourceSetCallbackEvents;

impl blaze_starlark::thread::StarlarkEvents for ResourceSetCallbackEvents {
    fn debug(&self, location: Option<&blaze_starlark::Location>, message: &str) {
        match location {
            Some(l) => eprintln!("{l}: {message}"),
            None => eprintln!(": {message}"),
        }
    }

    fn warn(&self, location: Option<&blaze_starlark::Location>, message: &str) {
        self.debug(location, message);
    }
}

/// The `resource_set` callback of `ctx.actions.run` / `run_shell`: called at execution with the
/// OS and the number of inputs, its dict result becomes the spawn's local resources.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:902 StarlarkActionResourceSetBuilder
pub(crate) struct StarlarkActionResourceSetBuilder {
    pub(crate) fn_: starlark::values::OwnedFrozenValue,
    pub(crate) mnemonic: String,
    pub(crate) semantics: Arc<StarlarkSemantics>,
}

impl fmt::Debug for StarlarkActionResourceSetBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StarlarkActionResourceSetBuilder({})", self.mnemonic)
    }
}

impl blaze_actions::analysis::actions::spawn_action::ResourceSetBuilder
    for StarlarkActionResourceSetBuilder
{
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:923 StarlarkActionResourceSetBuilder.buildResourceSet
    fn build_resource_set(
        &self,
        os: OS,
        inputs_size: i32,
    ) -> Result<ResourceSet, blaze_actions::analysis::actions::spawn_action::ResourceSetBuildError>
    {
        // Java: StarlarkThread.create(mu, semantics, "resource_set callback", ...): a thread
        // with the rule's semantics and no thread-locals.
        let ctx = blaze_starlark::ThreadContext::new(
            blaze_starlark::thread::Phase::Other,
            self.semantics.clone(),
            Arc::new(ResourceSetCallbackEvents),
        );
        let result = blaze_starlark::eval::call_frozen(
            &self.fn_,
            &ctx,
            &blaze_starlark::eval::EvalOptions::default(),
            |module| {
                let heap = module.heap();
                (
                    vec![
                        heap.alloc_str(os.get_canonical_name()).to_value(),
                        heap.alloc(inputs_size),
                    ],
                    Vec::new(),
                )
            },
            |response, _module, _eval| {
                resource_set_from_callback_response(response)
                    .map_err(|e| blaze_starlark::EvalError::from_starlark(&e))
            },
        );
        match result {
            Ok(resource_set) => Ok(resource_set),
            Err(e) => match e.kind {
                blaze_starlark::error::EvalErrorKind::Panic => panic!("{}", e.message),
                // Java: catch (EvalException e) -> UserExecException("Could not build
                // resources for %s. %s", mnemonic, e.getMessage()).
                _ => Err(
                    blaze_actions::analysis::actions::spawn_action::ResourceSetBuildError {
                        message: format!(
                            "Could not build resources for {}. {}",
                            self.mnemonic, e.message
                        ),
                    },
                ),
            },
        }
    }
}

/// The body of `buildResourceSet` after the call: `Dict.cast(response, ...)`, the key check and
/// `ResourceSet.create(memory, cpu, (int) local_test)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:934 StarlarkActionResourceSetBuilder.buildResourceSet
fn resource_set_from_callback_response(response: Value) -> starlark::Result<ResourceSet> {
    let resource_set_map_raw = dict_cast_string_object(response, "resource_set")?;
    let illegal: Vec<&str> = resource_set_map_raw
        .keys()
        .map(|k| k.as_str())
        .filter(|k| !VALID_RESOURCES.contains(k))
        .collect();
    if !illegal.is_empty() {
        return Err(errorf(format!(
            "Illegal resource keys: ({})",
            illegal.join(",")
        )));
    }
    let default = ResourceSet::default_resource_set();
    Ok(ResourceSet {
        memory_mb: get_numeric_or_default(&resource_set_map_raw, "memory", default.memory_mb)?,
        cpu_usage: get_numeric_or_default(&resource_set_map_raw, "cpu", default.cpu_usage)?,
        extra_resource_usage: IndexMap::new(),
        local_test_count: get_numeric_or_default(
            &resource_set_map_raw,
            "local_test",
            default.local_test_count as f64,
        )? as i32,
    })
}

/// Java `executableUnchecked` / `toolUnchecked` of type `FilesToRunProvider`.
fn files_to_run_provider(v: Value) -> Option<Arc<FilesToRunProvider>> {
    files_to_run_provider_from_value(v)
}

/// Java `x instanceof FilesToRunProvider ? (FilesToRunProvider) x : null`.
// PORT: `FilesToRunProvider` is a simple Starlark value (crate::providers::file_provider), copied
// into each heap, so the Rust provider is a clone.
pub fn files_to_run_provider_from_value(v: Value) -> Option<Arc<FilesToRunProvider>> {
    v.downcast_ref::<FilesToRunProvider>()
        .map(|p| Arc::new(p.clone()))
}

/// Allocates a `FilesToRunProvider` as a Starlark value.
pub fn files_to_run_provider_to_value<'v>(
    heap: Heap<'v>,
    provider: Arc<FilesToRunProvider>,
) -> Value<'v> {
    heap.alloc(provider.as_ref().clone())
}

/// Java's `FilesToRunProvider` identity (`equals` is `Object.equals`).
// PORT: Starlark copies of a provider are clones (see `files_to_run_provider_from_value`), so
// identity is approximated by the executable and the identity of the files-to-run set.
pub(crate) fn same_files_to_run_provider(a: &FilesToRunProvider, b: &FilesToRunProvider) -> bool {
    a.get_executable() == b.get_executable()
        && a.get_files_to_run()
            .shallow_equals(Some(b.get_files_to_run()))
}

/// The setup for spawn actions common to `run` and `run_shell` (Java `registerStarlarkAction`);
/// `builder` has its executable or command set.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:728 StarlarkActionFactory.registerStarlarkAction
fn register_starlark_action<'v>(
    f: &StarlarkActionFactory,
    outputs: Value<'v>,
    inputs: Value<'v>,
    unused_inputs_list: Value<'v>,
    tools_unchecked: Option<Value<'v>>,
    mnemonic_unchecked: Option<&str>,
    progress_message: Option<&str>,
    use_default_shell_env: bool,
    env_unchecked: Value<'v>,
    execution_requirements_unchecked: Value<'v>,
    exec_group_unchecked: Option<&str>,
    shadowed_action_unchecked: Value<'v>,
    resource_set_unchecked: Value<'v>,
    toolchain_unchecked: Option<Value<'v>>,
    mut builder: SpawnActionBuilder,
    command_line: Vec<CommandLinePart>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<()> {
    if is_sequence(inputs) {
        builder.add_inputs(sequence_cast_artifacts(inputs, "inputs", eval)?);
    } else {
        builder.add_transitive_inputs(&depset_cast_artifacts(inputs, "inputs")?);
    }

    let output_artifacts = sequence_cast_artifacts(outputs, "outputs", eval)?;
    if output_artifacts.is_empty() {
        return Err(errorf("param 'outputs' may not be empty"));
    }
    builder.add_outputs(output_artifacts);

    if !unused_inputs_list.is_none() {
        match artifact_from_value(unused_inputs_list) {
            Some(artifact) => set_unused_inputs_list(&mut builder, Some(artifact)),
            None => {
                return Err(errorf(format!(
                    "expected value of type 'File' for a member of parameter \
                     'unused_inputs_list' but got {} instead",
                    unused_inputs_list.get_type()
                )));
            }
        }
    }

    let handle = f.context.handle;
    let use_auto_exec_groups = handle.borrow().use_auto_exec_groups();

    if let Some(tools_unchecked) = tools_unchecked {
        let tools: Vec<Value<'v>> = if is_sequence(tools_unchecked) {
            sequence_elements(tools_unchecked, eval)?
        } else if Depset::from_value(tools_unchecked).is_some() {
            depset_to_list(tools_unchecked)?
        } else {
            return Err(errorf(format!(
                "for tools, got {}, want a depset of Object",
                tools_unchecked.get_type()
            )));
        };

        for tool_unchecked in tools {
            if let Some(artifact) = artifact_from_value(tool_unchecked) {
                builder.add_tool(artifact.clone());
                match f.context.get_executable_runfiles(&artifact, "executable")? {
                    Some(provider) => {
                        builder.add_transitive_tools(provider.get_files_to_run());
                    }
                    None => {
                        if use_auto_exec_groups && exec_group_unchecked.is_none() {
                            check_toolchain_parameter_is_set(
                                &handle.borrow(),
                                toolchain_unchecked,
                            )?;
                        }
                    }
                }
            } else if let Some(provider) = files_to_run_provider(tool_unchecked) {
                if use_auto_exec_groups
                    && !f.context.are_runfiles_from_deps(&provider)
                    && exec_group_unchecked.is_none()
                {
                    check_toolchain_parameter_is_set(&handle.borrow(), toolchain_unchecked)?;
                }
                builder.add_transitive_tools(provider.get_files_to_run());
            } else if Depset::from_value(tool_unchecked).is_some() {
                if use_auto_exec_groups && exec_group_unchecked.is_none() {
                    check_toolchain_parameter_is_set(&handle.borrow(), toolchain_unchecked)?;
                }
                let set = depset_get_set_artifacts(tool_unchecked).map_err(|e| {
                    errorf(format!(
                        "expected value of type 'File, FilesToRunProvider or Depset of Files' \
                         for a member of parameter 'tools' but {e}"
                    ))
                })?;
                builder.add_transitive_tools(&set);
            } else {
                return Err(errorf(format!(
                    "expected value of type 'File, FilesToRunProvider or Depset of Files' for a \
                     member of parameter 'tools' but got {} instead",
                    tool_unchecked.get_type()
                )));
            }
        }
    }

    let mnemonic = get_mnemonic(&handle.borrow(), mnemonic_unchecked, "Action");
    set_mnemonic(&mut builder, &mnemonic)?;
    if let Some(progress_message) = progress_message {
        builder.set_progress_message_from_starlark(progress_message);
    }

    let mut env: IndexMap<String, String> = IndexMap::new();
    if !env_unchecked.is_none() {
        env = dict_cast_string_string(env_unchecked, "env")?;
    }
    // Java: Starlark.truth(useDefaultShellEnv)
    if use_default_shell_env {
        builder.use_default_shell_environment(env);
    } else {
        builder.set_environment(env);
    }

    let execution_info = {
        let rc = handle.borrow();
        let allow = f
            .context
            .get_starlark_semantics()
            .incompatible_allow_tags_propagation;
        get_filtered_execution_info(Some(execution_requirements_unchecked), rc.get_rule(), allow)?
    };
    builder.set_execution_info(execution_info.into_iter().collect());

    let exec_group = determine_exec_group(
        &handle.borrow(),
        exec_group_unchecked,
        toolchain_unchecked,
        eval,
    )?;
    builder.set_exec_group(exec_group);

    if !shadowed_action_unchecked.is_none() {
        // Java: builder.setShadowedAction(Optional.of((Action) shadowedActionUnchecked))
        blaze_util::unported!("StarlarkActionFactory.registerStarlarkAction(shadowed_action)");
    }

    let mut resource_set_callback = None;
    if !resource_set_unchecked.is_none() {
        if DictRef::from_value(resource_set_unchecked).is_some() {
            builder.set_resources(parse_resource_set_from_dict(resource_set_unchecked)?);
        } else {
            // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1065 StarlarkActionFactory.validateResourceSetBuilder
            if !matches!(
                resource_set_unchecked.get_type(),
                "function" | "builtin_function_or_method"
            ) {
                return Err(errorf(format!(
                    "resource_set should be a Starlark-callable function, but got {} instead",
                    resource_set_unchecked.get_type()
                )));
            }
            // Java: `fn instanceof StarlarkFunction sfn` (a `def` or `lambda`).
            if blaze_starlark::defining_module::module_file_of_function(resource_set_unchecked)
                .is_some()
            {
                validate_is_top_level_starlark_function(resource_set_unchecked, eval)?;
            }
            // Java: builder.setResources(StarlarkActionResourceSetBuilder.create(
            // (StarlarkCallable) resourceSetUnchecked, mnemonic, getSemantics())): the callable
            // is kept with the module and called at execution (`finish_pending_actions` builds
            // the `StarlarkActionResourceSetBuilder` once the module is frozen).
            resource_set_callback = Some((
                keep_alive_push(eval, resource_set_unchecked)?,
                mnemonic.clone(),
                handle.borrow().get_starlark_semantics().clone(),
            ));
        }
    }

    // Always register the action
    register_action(
        f,
        PendingAction::Spawn {
            builder: Box::new(builder),
            command_line,
            resource_set_callback,
        },
        eval,
    )
}

/// Rejects a `def` that is not its module's global of the same name (a nested def or a
/// lambda).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1077 StarlarkActionFactory.validateIsTopLevelStarlarkFunction
fn validate_is_top_level_starlark_function<'v>(
    function: Value<'v>,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<()> {
    // Reject non-global functions, because arbitrary closures may cause large
    // analysis-phase data structures to remain live into the execution phase.
    // We require that the function is "global" as opposed to "not a closure"
    // because a global function may be closure if it refers to load bindings.
    // This unfortunately disallows such trivially safe non-global
    // functions as "lambda x: x".
    // See https://github.com/bazelbuild/bazel/issues/12701.
    if !blaze_starlark::defining_module::is_global_of_defining_module(function, eval) {
        // Java: "to avoid unintended retention of analysis data structures, the function
        // (declared at %s) must be declared by a top-level def statement", fn.getLocation().
        // starlark-rust exposes no declaration location of a function value.
        blaze_util::unported!(
            "StarlarkFunction.getLocation",
            "location of a non-top-level resource_set function"
        );
    }
    Ok(())
}

/// `toolchain`'s `unbound` default: `None` when the parameter was not passed. Fails like the
/// interpreter when the argument is not a Label, a string or None.
// Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkActionFactoryApi.java:597 StarlarkActionFactoryApi.run (@Param toolchain)
fn toolchain_param<'v>(
    method_name: &str,
    toolchain: Option<Value<'v>>,
) -> starlark::Result<Option<Value<'v>>> {
    if let Some(t) = toolchain {
        if !(t.is_none() || t.downcast_ref::<LabelValue>().is_some() || t.unpack_str().is_some()) {
            return Err(param_type_error(
                method_name,
                "toolchain",
                t,
                "Label, string, or NoneType",
            ));
        }
    }
    Ok(toolchain)
}

/// Java `x instanceof Sequence || x instanceof Depset`.
fn is_sequence_or_depset(v: Value) -> bool {
    is_sequence(v) || Depset::from_value(v).is_some()
}

/// Java `x instanceof Dict || x == Starlark.NONE`.
fn is_dict_or_none(v: Value) -> bool {
    v.is_none() || DictRef::from_value(v).is_some()
}

/// Java `x instanceof StarlarkCallable`, as far as `resource_set` is concerned (see
/// `validateResourceSetBuilder` in `register_starlark_action`).
fn is_starlark_callable(v: Value) -> bool {
    matches!(v.get_type(), "function" | "builtin_function_or_method")
}

/// The interpreter's type checks of the `run` / `run_shell` arguments that both methods declare
/// with the same `@Param` types and that the Rust signatures take as plain values. The
/// interpreter checks arguments in declaration order before the method body runs, so the
/// callers interleave these with the checks of their own parameters.
// Java: net/starlark/java/eval/BuiltinFunction.java:405 BuiltinFunction.checkParamValue
// Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkActionFactoryApi.java:399 StarlarkActionFactoryApi.run
// Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkActionFactoryApi.java:637 StarlarkActionFactoryApi.runShell
fn check_outputs_and_inputs_params<'v>(
    method_name: &str,
    outputs: Value<'v>,
    inputs: Value<'v>,
) -> starlark::Result<()> {
    if !is_sequence(outputs) {
        return Err(param_type_error(
            method_name,
            "outputs",
            outputs,
            "sequence",
        ));
    }
    if !is_sequence_or_depset(inputs) {
        return Err(param_type_error(
            method_name,
            "inputs",
            inputs,
            "sequence or depset",
        ));
    }
    Ok(())
}

/// See [`check_outputs_and_inputs_params`]: `tools` and `arguments`.
fn check_tools_and_arguments_params<'v>(
    method_name: &str,
    tools: Option<Value<'v>>,
    arguments: Value<'v>,
) -> starlark::Result<()> {
    if let Some(tools) = tools {
        if !is_sequence_or_depset(tools) {
            return Err(param_type_error(
                method_name,
                "tools",
                tools,
                "sequence or depset",
            ));
        }
    }
    if !is_sequence(arguments) {
        return Err(param_type_error(
            method_name,
            "arguments",
            arguments,
            "sequence",
        ));
    }
    Ok(())
}

/// See [`check_outputs_and_inputs_params`]: the parameters from `use_default_shell_env` to
/// `toolchain`. Returns `use_default_shell_env` and `toolchain` (`None` when unbound).
fn check_trailing_spawn_params<'v>(
    method_name: &str,
    use_default_shell_env: Value<'v>,
    env: Value<'v>,
    execution_requirements: Value<'v>,
    input_manifests: Value<'v>,
    resource_set: Value<'v>,
    toolchain: Option<Value<'v>>,
) -> starlark::Result<(bool, Option<Value<'v>>)> {
    let use_default_shell_env =
        check_bool_param(method_name, "use_default_shell_env", use_default_shell_env)?;
    if !is_dict_or_none(env) {
        return Err(param_type_error(
            method_name,
            "env",
            env,
            "dict or NoneType",
        ));
    }
    if !is_dict_or_none(execution_requirements) {
        return Err(param_type_error(
            method_name,
            "execution_requirements",
            execution_requirements,
            "dict or NoneType",
        ));
    }
    if !(input_manifests.is_none() || is_sequence(input_manifests)) {
        return Err(param_type_error(
            method_name,
            "input_manifests",
            input_manifests,
            "sequence or NoneType",
        ));
    }
    if !(is_dict_or_none(resource_set) || is_starlark_callable(resource_set)) {
        return Err(param_type_error(
            method_name,
            "resource_set",
            resource_set,
            "callable, dict, or NoneType",
        ));
    }
    let toolchain = toolchain_param(method_name, toolchain)?;
    Ok((use_default_shell_env, toolchain))
}

/// Java `CommandHelper.commandHelperScriptMaybe(ruleContext, command, constructor,
/// executionOs)` as `run_shell` uses it: if `command` is too long, the helper shell script that
/// runs it and the `FileWriteAction` that writes it; otherwise `None`.
///
/// PORT: Java's `FileWriteAction.createFile` registers the action with the rule context. The
/// actions of a Starlark rule are registered through the ordered pending list (module docs), so
/// the action is returned for the caller to put there, in place. `script_name_suffix` is the
/// suffix Java passes through its `BashCommandConstructor`, whose shell path this code path does
/// not read.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:281 CommandHelper.commandHelperScriptMaybe
// Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:44 BashCommandConstructor.commandAsScript
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:84 FileWriteAction.createFile
fn command_helper_script_maybe(
    rule_context: &RuleContext,
    command: &str,
    script_name_suffix: &str,
    execution_os: OS,
) -> Option<(Artifact, FileWriteAction)> {
    // Java: `command.length()` of an internal string: its UTF-8 bytes (analysis review R2-5).
    if crate::rule::command_helper::java_string_length(command)
        <= crate::rule::command_helper::max_command_length(execution_os)
    {
        return None;
    }
    let script_file_name = format!(
        "{}{}",
        rule_context.get_target().get_name(),
        script_name_suffix
    );
    let script_file_contents = format!("#!/bin/bash\n{command}");
    let script_file_artifact = rule_context.get_package_relative_artifact(
        &PathFragment::create(&script_file_name),
        &rule_context.get_genfiles_directory(),
    );
    let action = FileWriteAction::create_from_context(
        rule_context.get_action_owner(),
        script_file_artifact.clone(),
        script_file_contents,
        /* make_executable= */ true,
    );
    Some((script_file_artifact, action))
}

#[starlark_module]
fn actions_methods(builder: &mut MethodsBuilder) {
    /// Declares that the rule or aspect creates a file with the given filename.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:143 StarlarkActionFactory.declareFile
    fn declare_file<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] filename: &str,
        #[starlark(require = named, default = NoneOr::None)] sibling: NoneOr<&StarlarkFile>,
    ) -> starlark::Result<StarlarkFile> {
        let f = factory(this, "actions.declare_file")?;
        let root = f.context.new_file_root();
        let sibling_layout = f
            .context
            .get_starlark_semantics()
            .experimental_sibling_repository_layout;
        let rule_context = f.context.handle.borrow_mut();
        let fragment = declared_path(
            &rule_context,
            filename,
            sibling.into_option().map(|s| s.artifact()),
            sibling_layout,
        );
        let package_directory = rule_context.get_package_directory();
        if !fragment.starts_with(&package_directory) {
            return Err(errorf(format!(
                "the output artifact '{}' is not under package directory '{}' for target '{}'",
                fragment,
                package_directory,
                rule_context.get_label()
            )));
        }
        Ok(StarlarkFile::new(
            rule_context.get_derived_artifact(&fragment, &root),
        ))
    }

    /// Declares that the rule or aspect creates a directory with the given name.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:166 StarlarkActionFactory.declareDirectory
    fn declare_directory<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] filename: &str,
        #[starlark(require = named, default = NoneOr::None)] sibling: NoneOr<&StarlarkFile>,
    ) -> starlark::Result<StarlarkFile> {
        let f = factory(this, "actions.declare_directory")?;
        let root = f.context.new_file_root();
        let sibling_layout = f
            .context
            .get_starlark_semantics()
            .experimental_sibling_repository_layout;
        let rule_context = f.context.handle.borrow_mut();
        let fragment = declared_path(
            &rule_context,
            filename,
            sibling.into_option().map(|s| s.artifact()),
            sibling_layout,
        );
        let package_directory = rule_context.get_package_directory();
        if !fragment.starts_with(&package_directory) {
            return Err(errorf(format!(
                "the output directory '{}' is not under package directory '{}' for target '{}'",
                fragment,
                package_directory,
                rule_context.get_label()
            )));
        }
        let result = rule_context.get_tree_artifact(&fragment, &root);
        if !result.is_tree_artifact() {
            return Err(errorf(format!(
                "'{filename}' has already been declared as a regular file, not directory."
            )));
        }
        Ok(StarlarkFile::new(result))
    }

    /// Declares that the rule or aspect creates a symlink with the given name.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:194 StarlarkActionFactory.declareSymlink
    fn declare_symlink<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] filename: &str,
        #[starlark(require = named, default = NoneOr::None)] sibling: NoneOr<&StarlarkFile>,
    ) -> starlark::Result<StarlarkFile> {
        let f = factory(this, "actions.declare_symlink")?;
        let root = f.context.new_file_root();
        let sibling_layout = f
            .context
            .get_starlark_semantics()
            .experimental_sibling_repository_layout;
        let rule_context = f.context.handle.borrow();
        if !allow_unresolved_symlinks(rule_context.get_configuration()) {
            return Err(errorf(
                "actions.declare_symlink() is not allowed; use the --allow_unresolved_symlinks \
                 command line option",
            ));
        }
        let root_relative_path = declared_path(
            &rule_context,
            filename,
            sibling.into_option().map(|s| s.artifact()),
            sibling_layout,
        );
        let result = rule_context
            .get_analysis_environment()
            .get_symlink_artifact(&root_relative_path, &root);
        if !result.is_symlink() {
            return Err(errorf(format!(
                "'{filename}' has already been declared as something other than a symlink."
            )));
        }
        Ok(StarlarkFile::new(result))
    }

    /// Creates an empty action that neither executes a command nor produces any output.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:230 StarlarkActionFactory.doNothing
    fn do_nothing<'v>(
        this: Value<'v>,
        #[starlark(require = named)] mnemonic: &str,
        #[starlark(require = named, default = starlark::values::list::AllocList::EMPTY)]
        inputs: Value<'v>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        let _f = factory(this, "actions.do_nothing")?;
        let _ = (mnemonic, inputs);
        // Java: new PseudoAction<>(...) with a SpawnInfo extra action info.
        blaze_util::unported!("StarlarkActionFactory.doNothing")
    }

    /// Creates an action that writes a symlink in the file system.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:258 StarlarkActionFactory.symlink
    fn symlink<'v>(
        this: Value<'v>,
        #[starlark(require = named)] output: &StarlarkFile,
        #[starlark(require = named, default = NoneOr::None)] target_file: NoneOr<&StarlarkFile>,
        #[starlark(require = named, default = NoneOr::None)] target_path: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] target_type: NoneOr<&str>,
        #[starlark(require = named, default = false)] is_executable: bool,
        #[starlark(require = named, default = NoneOr::None)] progress_message: NoneOr<&str>,
        #[starlark(require = named)] use_exec_root_for_source: Option<bool>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        let f = factory(this, "actions.symlink")?;
        if use_exec_root_for_source.is_some() {
            fail_if_called_outside_default_allowlist(eval)?;
        }
        let use_exec_root_for_source = use_exec_root_for_source.unwrap_or(false);

        let target_file = target_file.into_option();
        let target_path = target_path.into_option();
        if target_file.is_none() == target_path.is_none() {
            return Err(errorf(
                "Exactly one of \"target_file\" or \"target_path\" is required",
            ));
        }

        let output_artifact = output.artifact().clone();
        let progress_message = progress_message
            .into_option()
            .unwrap_or("Creating symlink %{output}")
            .to_owned();

        let owner = f.context.handle.borrow_mut().get_action_owner();
        let action: ActionRef = if let Some(target_file) = target_file {
            if target_type.into_option().is_some() {
                return Err(errorf(
                    "\"target_type\" cannot be used with \"target_file\"",
                ));
            }
            let input_artifact = target_file.artifact().clone();
            if output_artifact.is_symlink() {
                return Err(errorf(
                    "symlink() with \"target_file\" param requires that \"output\" be declared \
                     as a file or directory, not a symlink (did you mean to use declare_file() \
                     or declare_directory() instead of declare_symlink()?)",
                ));
            }
            if input_artifact.is_directory() != output_artifact.is_directory() {
                let input_type = if input_artifact.is_directory() {
                    "directory"
                } else {
                    "file"
                };
                let output_type = if output_artifact.is_directory() {
                    "directory"
                } else {
                    "file"
                };
                return Err(errorf(format!(
                    "symlink() with \"target_file\" {input_type} param requires that \"output\" \
                     be declared as a {input_type} (did you mean to use declare_{input_type}() \
                     instead of declare_{output_type}()?)"
                )));
            }
            if is_executable {
                if output_artifact.is_tree_artifact() {
                    return Err(errorf(
                        "symlink() with \"output\" directory param cannot be executable",
                    ));
                }
                Arc::new(SymlinkAction::to_executable(
                    owner,
                    input_artifact,
                    output_artifact,
                    Some(progress_message),
                ))
            } else {
                Arc::new(SymlinkAction::to_artifact_with_exec_root(
                    owner,
                    input_artifact,
                    output_artifact,
                    Some(progress_message),
                    use_exec_root_for_source,
                ))
            }
        } else {
            if !output_artifact.is_symlink() {
                return Err(errorf(
                    "symlink() with \"target_path\" param requires that \"output\" be declared \
                     as a symlink, not a file or directory (did you mean to use \
                     declare_symlink() instead of declare_file() or declare_directory()?)",
                ));
            }
            if is_executable {
                return Err(errorf(
                    "\"is_executable\" cannot be True when using \"target_path\"",
                ));
            }
            let symlink_target_type = match target_type.into_option() {
                None => SymlinkTargetType::UNSPECIFIED,
                Some("file") => SymlinkTargetType::FILE,
                Some("directory") => SymlinkTargetType::DIRECTORY,
                Some(_) => {
                    return Err(errorf(
                        "\"target_type\" must be one of \"file\" or \"directory\"",
                    ));
                }
            };
            Arc::new(UnresolvedSymlinkAction::create(
                owner,
                output_artifact,
                target_path.expect("target_path"),
                symlink_target_type,
                progress_message,
            ))
        };
        register_action(f, PendingAction::Built(action), eval)?;
        Ok(starlark::values::none::NoneType)
    }

    /// Creates a file write action.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:375 StarlarkActionFactory.write
    fn write<'v>(
        this: Value<'v>,
        output: &StarlarkFile,
        content: Value<'v>,
        #[starlark(default = false)] is_executable: bool,
        #[starlark(require = named, default = NoneOr::None)] mnemonic: NoneOr<&str>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        execution_requirements: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        let f = factory(this, "actions.write")?;
        let handle = f.context.handle;
        let mnemonic = get_mnemonic(
            &handle.borrow(),
            mnemonic.into_option(),
            abstract_file_write_action::MNEMONIC,
        );

        let action = if let Some(content) = content.unpack_str() {
            let owner = handle.borrow_mut().get_action_owner();
            // Java: FileWriteAction.create(ruleContext, output, content, isExecutable, mnemonic)
            PendingAction::Built(Arc::new(
                FileWriteAction::create_from_context_with_mnemonic(
                    owner,
                    output.artifact().clone(),
                    content.to_owned(),
                    is_executable,
                    &mnemonic,
                ),
            ))
        } else if is_args(content) {
            let allow = f
                .context
                .get_starlark_semantics()
                .incompatible_allow_tags_propagation;
            let unmodified_execution_requirements = get_filtered_execution_info(
                Some(execution_requirements),
                handle.borrow().get_rule(),
                allow,
            )?;
            let owner = handle.borrow_mut().get_action_owner();
            // Java: args.getDirectoryArtifacts(), args.build(getMainRepoMappingSupplier()) and
            // args.getParameterFileType() are read here: a copy of the Args state as it is now
            // (see `build_command_line`).
            let snapshot = snapshot_args(content, eval.heap());
            PendingAction::ParameterFileWrite {
                owner,
                output: output.artifact().clone(),
                args: keep_alive_push(eval, snapshot)?,
                is_executable,
                mnemonic,
                unmodified_execution_requirements: unmodified_execution_requirements
                    .into_iter()
                    .collect(),
            }
        } else {
            // Java: @ParamType(String, CommandLineArgsApi) is checked by the interpreter.
            return Err(errorf(format!(
                "in call to write(), parameter 'content' got value of type '{}', want 'string or \
                 Args'",
                content.get_type()
            )));
        };
        register_action(f, action, eval)?;
        Ok(starlark::values::none::NoneType)
    }

    /// Creates an action that runs an executable.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:415 StarlarkActionFactory.run
    fn run<'v>(
        this: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        #[starlark(require = named, default = starlark::values::list::AllocList::EMPTY)]
        inputs: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        unused_inputs_list: Value<'v>,
        #[starlark(require = named)] executable: Value<'v>,
        #[starlark(require = named)] tools: Option<Value<'v>>,
        #[starlark(require = named, default = starlark::values::list::AllocList::EMPTY)]
        arguments: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] mnemonic: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] progress_message: NoneOr<&str>,
        #[starlark(require = named, default = false)] use_default_shell_env: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)] env: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        execution_requirements: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        input_manifests: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] exec_group: NoneOr<&str>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        shadowed_action: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        resource_set: Value<'v>,
        #[starlark(require = named)] toolchain: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        // The interpreter checks every argument against its `@Param` before the method runs,
        // in declaration order (`mnemonic`, `progress_message` and `exec_group` are checked by
        // their Rust parameter types).
        check_outputs_and_inputs_params("run", outputs, inputs)?;
        if !(unused_inputs_list.is_none() || artifact_from_value(unused_inputs_list).is_some()) {
            return Err(param_type_error(
                "run",
                "unused_inputs_list",
                unused_inputs_list,
                "File or NoneType",
            ));
        }
        if !(artifact_from_value(executable).is_some()
            || executable.unpack_str().is_some()
            || files_to_run_provider(executable).is_some())
        {
            return Err(param_type_error(
                "run",
                "executable",
                executable,
                "File, string, or FilesToRunProvider",
            ));
        }
        check_tools_and_arguments_params("run", tools, arguments)?;
        let (use_default_shell_env, toolchain) = check_trailing_spawn_params(
            "run",
            use_default_shell_env,
            env,
            execution_requirements,
            input_manifests,
            resource_set,
            toolchain,
        )?;

        let f = factory(this, "actions.run")?;
        let exec_group = f
            .context
            .maybe_override_exec_group(exec_group.into_option())?;
        let toolchain = f.context.maybe_override_toolchain(toolchain, eval.heap())?;

        let handle = f.context.handle;
        let use_auto_exec_groups = handle.borrow().use_auto_exec_groups();

        let mut builder = starlark_action_builder();
        let arguments = sequence_elements(arguments, eval)?;
        let command_line = build_command_line(&arguments, eval)?;
        if let Some(executable) = artifact_from_value(executable) {
            match f
                .context
                .get_executable_runfiles(&executable, "executable")?
            {
                None => {
                    if use_auto_exec_groups && exec_group.is_none() {
                        check_toolchain_parameter_is_set(&handle.borrow(), toolchain)?;
                    }
                    builder.set_executable(executable);
                }
                Some(provider) => {
                    builder.set_executable_files_to_run(
                        provider.get_executable().cloned(),
                        provider.get_files_to_run(),
                    );
                }
            }
        } else if let Some(s) = executable.unpack_str() {
            // Normalise if needed and then pass as a String; this keeps the reference when
            // PathFragment is passed from native to Starlark
            builder.set_executable_as_string(PathFragment::create(s).get_path_string());
        } else if let Some(provider) = files_to_run_provider(executable) {
            if use_auto_exec_groups
                && !f.context.are_runfiles_from_deps(&provider)
                && exec_group.is_none()
            {
                check_toolchain_parameter_is_set(&handle.borrow(), toolchain)?;
            }
            builder.set_executable_files_to_run(
                provider.get_executable().cloned(),
                provider.get_files_to_run(),
            );
        } else {
            // Should have been verified by Starlark before this function is called
            panic!("IllegalStateException");
        }
        register_starlark_action(
            f,
            outputs,
            inputs,
            unused_inputs_list,
            tools,
            mnemonic.into_option(),
            progress_message.into_option(),
            use_default_shell_env,
            env,
            execution_requirements,
            exec_group,
            shadowed_action,
            resource_set,
            toolchain,
            builder,
            command_line,
            eval,
        )?;
        Ok(starlark::values::none::NoneType)
    }

    /// Creates an action that runs a shell command.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:600 StarlarkActionFactory.runShell
    fn run_shell<'v>(
        this: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        #[starlark(require = named, default = starlark::values::list::AllocList::EMPTY)]
        inputs: Value<'v>,
        #[starlark(require = named)] tools: Option<Value<'v>>,
        #[starlark(require = named, default = starlark::values::list::AllocList::EMPTY)]
        arguments: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] mnemonic: NoneOr<&str>,
        #[starlark(require = named)] command: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] progress_message: NoneOr<&str>,
        #[starlark(require = named, default = false)] use_default_shell_env: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)] env: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        execution_requirements: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        input_manifests: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] exec_group: NoneOr<&str>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        shadowed_action: Value<'v>,
        #[starlark(require = named, default = starlark::values::none::NoneType)]
        resource_set: Value<'v>,
        #[starlark(require = named)] toolchain: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        // The interpreter checks every argument against its `@Param` before the method runs,
        // in declaration order (`mnemonic`, `progress_message` and `exec_group` are checked by
        // their Rust parameter types).
        check_outputs_and_inputs_params("run_shell", outputs, inputs)?;
        check_tools_and_arguments_params("run_shell", tools, arguments)?;
        if !(command.unpack_str().is_some() || is_sequence(command)) {
            return Err(param_type_error(
                "run_shell",
                "command",
                command,
                "string or sequence",
            ));
        }
        let (use_default_shell_env, toolchain) = check_trailing_spawn_params(
            "run_shell",
            use_default_shell_env,
            env,
            execution_requirements,
            input_manifests,
            resource_set,
            toolchain,
        )?;

        let f = factory(this, "actions.run_shell")?;
        let exec_group = f
            .context
            .maybe_override_exec_group(exec_group.into_option())?;
        let toolchain = f.context.maybe_override_toolchain(toolchain, eval.heap())?;
        let handle = f.context.handle;

        let mut builder = starlark_action_builder();
        let arguments = sequence_elements(arguments, eval)?;
        let command_line = build_command_line(&arguments, eval)?;

        // When we use a shell command, add an empty argument before other arguments.
        //   e.g.  bash -c "cmd" '' 'arg1' 'arg2'
        // bash will use the empty argument as the value of $0 (which we don't care about).
        // arg1 and arg2 will be $1 and $2, as a user expects.
        let pad = !arguments.is_empty();

        if let Some(command) = command.unpack_str() {
            let (sh_executable, helper_script) = {
                let rule_context = handle.borrow();
                // Java: `executionInfo` (TargetUtils.getExecutionInfo(ruleContext.getRule()))
                // only chooses the shell path of the BashCommandConstructor, which
                // commandHelperScriptMaybe does not read (see `command_helper_script_maybe`).
                let counter = f.run_shell_output_counter.get();
                f.run_shell_output_counter.set(counter + 1);
                let helper_script_suffix = format!(".run_shell_{counter}.sh");
                let execution_platform = get_execution_platform(exec_group, &rule_context)?;
                let sh_executable = crate::rule::sh_toolchain::get_path_for_platform(
                    rule_context.get_configuration(),
                    get_execution_platform(exec_group, &rule_context)?.as_ref(),
                );
                let helper_script = command_helper_script_maybe(
                    &rule_context,
                    command,
                    &helper_script_suffix,
                    crate::platform::get_os_from_constraints_or_host(execution_platform.as_deref()),
                );
                (sh_executable, helper_script)
            };
            match helper_script {
                None => {
                    builder.set_shell_command(&sh_executable, command, pad);
                }
                Some((helper_script, helper_script_action)) => {
                    // Java: FileWriteAction.createFile calls ruleContext.registerAction(action)
                    // here: after the actions registered so far, before this run_shell action.
                    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:640 StarlarkActionFactory.runShell
                    AnalysisThreadPayload::from_or_fail(eval, "registerAction")?
                        .push_pending(PendingAction::Built(Arc::new(helper_script_action)));
                    builder.set_shell_command(
                        &sh_executable,
                        helper_script.get_exec_path_string(),
                        pad,
                    );
                    builder.add_input(helper_script);
                }
            }
        } else if is_sequence(command) {
            if f.context
                .get_starlark_semantics()
                .incompatible_run_shell_command_string
            {
                return Err(errorf(
                    "'command' must be of type string. passing a sequence of strings as \
                     'command' is deprecated. To temporarily disable this check, set \
                     --incompatible_run_shell_command_string=false.",
                ));
            }
            if !arguments.is_empty() {
                return Err(errorf(
                    "'arguments' must be empty if 'command' is a sequence of strings",
                ));
            }
            let command = sequence_cast_strings(command, "command", eval)?;
            builder.set_shell_command_argv(command, pad);
        } else {
            return Err(errorf(format!(
                "expected string or list of strings for command instead of {}",
                command.get_type()
            )));
        }
        register_starlark_action(
            f,
            outputs,
            inputs,
            /* unused_inputs_list= */ Value::new_none(),
            tools,
            mnemonic.into_option(),
            progress_message.into_option(),
            use_default_shell_env,
            env,
            execution_requirements,
            exec_group,
            shadowed_action,
            resource_set,
            toolchain,
            builder,
            command_line,
            eval,
        )?;
        Ok(starlark::values::none::NoneType)
    }

    /// Creates a template expansion action.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1220 StarlarkActionFactory.expandTemplate
    fn expand_template<'v>(
        this: Value<'v>,
        #[starlark(require = named)] template: &StarlarkFile,
        #[starlark(require = named)] output: &StarlarkFile,
        #[starlark(require = named, default = starlark::values::dict::AllocDict::EMPTY)]
        substitutions: Value<'v>,
        #[starlark(require = named, default = false)] is_executable: bool,
        #[starlark(require = named)] computed_substitutions: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        let f = factory(this, "actions.expand_template")?;
        // We use a map to check for duplicate keys
        let mut substitutions_builder: Vec<Substitution> = Vec::new();
        for (k, v) in dict_cast_string_string(substitutions, "substitutions")? {
            substitutions_builder.push(Substitution::of(k, v));
        }
        let owner = f.context.handle.borrow_mut().get_action_owner();
        let action = match computed_substitutions {
            None => PendingAction::Built(Arc::new(TemplateExpansionAction::for_template_artifact(
                owner,
                template.artifact().clone(),
                output.artifact().clone(),
                substitutions_builder,
                is_executable,
            ))),
            Some(computed) => {
                if !is_template_dict(computed) {
                    return Err(errorf(format!(
                        "in call to expand_template(), parameter 'computed_substitutions' got \
                         value of type '{}', want 'TemplateDict'",
                        computed.get_type()
                    )));
                }
                // Java: the substitutions of `((TemplateDict) computedSubstitutions).getAll()`
                // are put into the action's list here, so `add` / `add_joined` called afterwards
                // do not reach this action: a copy of the TemplateDict as it is now is kept.
                // PORT: Java also rejects a key that repeats one of `substitutions` here
                // ("Multiple entries with same key: ..."); the keys of a TemplateDict can only
                // be read once it is frozen, so that case is reported (loudly) by
                // `finish_pending_actions`.
                let snapshot = snapshot_template_dict(computed, eval.heap());
                PendingAction::TemplateExpansion {
                    owner,
                    template: template.artifact().clone(),
                    output: output.artifact().clone(),
                    substitutions: substitutions_builder,
                    computed_substitutions: keep_alive_push(eval, snapshot)?,
                    executable: is_executable,
                }
            }
        };
        register_action(f, action, eval)?;
        Ok(starlark::values::none::NoneType)
    }

    /// Returns an Args object that can be used to build memory-efficient command lines.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1256 StarlarkActionFactory.args
    fn args<'v>(this: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<Value<'v>> {
        let semantics = match this.downcast_ref::<StarlarkActionFactory>() {
            Some(f) => f.context.get_starlark_semantics(),
            None => blaze_starlark::ThreadContext::get(eval)
                .map_err(starlark::Error::new_other)?
                .semantics
                .clone(),
        };
        Ok(new_args(eval.heap(), semantics))
    }

    /// Returns a TemplateDict object for memory-efficient template expansion.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1261 StarlarkActionFactory.templateDict
    fn template_dict<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let _ = this;
        Ok(new_template_dict(heap))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1266 StarlarkActionFactory.createShareableArtifact
    fn declare_shareable_artifact<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = pos)] artifact_root: Option<&StarlarkFileRoot>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        fail_if_called_outside_default_allowlist(eval)?;
        let f = match this.downcast_ref::<StarlarkActionFactory>() {
            Some(f) => f,
            None => return Err(factory(this, "actions.declare_shareable_artifact").unwrap_err()),
        };
        let rule_context = f.context.handle.borrow_mut();
        let root = match artifact_root {
            None => rule_context.get_bin_directory(),
            Some(StarlarkFileRoot(FileRoot::Artifact(root))) => root.clone(),
            Some(_) => {
                return Err(errorf(
                    "in call to declare_shareable_artifact(), parameter 'artifact_root' got a \
                     mapped root, want an ArtifactRoot",
                ));
            }
        };
        Ok(StarlarkFile::new(rule_context.get_shareable_artifact(
            &PathFragment::create(path),
            &root,
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:465 StarlarkActionFactory.transformVersionFile
    fn transform_version_file<'v>(
        this: Value<'v>,
        #[starlark(require = named)] transform_func: Value<'v>,
        #[starlark(require = named)] template: &StarlarkFile,
        #[starlark(require = named)] output_file_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        fail_if_called_outside_default_allowlist(eval)?;
        let f = factory(this, "actions.transform_version_file")?;
        transform_build_info_file(f, transform_func, template, output_file_name, true, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:477 StarlarkActionFactory.transformInfoFile
    fn transform_info_file<'v>(
        this: Value<'v>,
        #[starlark(require = named)] transform_func: Value<'v>,
        #[starlark(require = named)] template: &StarlarkFile,
        #[starlark(require = named)] output_file_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        fail_if_called_outside_default_allowlist(eval)?;
        let f = factory(this, "actions.transform_info_file")?;
        transform_build_info_file(f, transform_func, template, output_file_name, false, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1138 StarlarkActionFactory.mapDirectory
    fn map_directory<'v>(
        this: Value<'v>,
        #[starlark(kwargs)] kwargs: Value<'v>,
    ) -> starlark::Result<starlark::values::none::NoneType> {
        let _ = (this, kwargs);
        blaze_util::unported!("StarlarkActionFactory.mapDirectory")
    }
}

// -------------------------------------------------------------------------------------------
// Build info files (hook installed by blaze_rules)

/// Builds the `BuildInfoFileWriteAction` of a `transform_*_file` call from the frozen
/// translation function.
pub type BuildInfoFileWriteActionBuilder = Box<dyn FnOnce(OwnedFrozenValue) -> ActionRef + Send>;

/// Java `StarlarkActionFactory.transformBuildInfoFile` up to `registerAction`: declares the
/// output artifact and describes the action (blaze_rules' `build_info`, which this crate cannot
/// depend on). Arguments: rule context, `newFileRoot()`, `transform_func`, the module context
/// of the module `transform_func` was defined in (Java
/// `BazelModuleContext.of(translationFunc.getModule())`), `template`, `output_file_name`,
/// `is_volatile`.
pub type TransformBuildInfoFile = for<'v> fn(
    &RuleContext,
    &ArtifactRoot,
    Value<'v>,
    Option<Arc<BazelModuleContext>>,
    &Artifact,
    &str,
    bool,
) -> (Artifact, BuildInfoFileWriteActionBuilder);

static TRANSFORM_BUILD_INFO_FILE: std::sync::OnceLock<TransformBuildInfoFile> =
    std::sync::OnceLock::new();

/// Installs the [`TransformBuildInfoFile`] hook (once, before analysis).
pub fn set_transform_build_info_file(hook: TransformBuildInfoFile) {
    let _ = TRANSFORM_BUILD_INFO_FILE.set(hook);
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:500 StarlarkActionFactory.transformBuildInfoFile
fn transform_build_info_file<'v>(
    f: &StarlarkActionFactory,
    transform_func: Value<'v>,
    template: &StarlarkFile,
    output_file_name: &str,
    is_volatile: bool,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<StarlarkFile> {
    let Some(hook) = TRANSFORM_BUILD_INFO_FILE.get() else {
        blaze_util::unported!(
            "StarlarkActionFactory.transformBuildInfoFile",
            "the build info hook is not installed"
        );
    };
    let new_file_root = f.context.new_file_root();
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:196 BuildInfoFileWriteAction.computeKey
    // `BazelModuleContext.of(translationCallback.getModule())`, read here because the digest is
    // stored on the action spec; `None` when `transform_func` is not a Starlark function (the
    // hook's cast fails first).
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:142 BazelModuleContext.of
    let function_module = BazelModuleContext::of_function(transform_func, eval);
    let (build_info_file, build) = {
        let rule_context = f.context.handle.borrow();
        hook(
            &rule_context,
            &new_file_root,
            transform_func,
            function_module,
            template.artifact(),
            output_file_name,
            is_volatile,
        )
    };
    // Java: ruleContext.registerAction(action), in this call's place in the registration order.
    let translation_callback = keep_alive_push(eval, transform_func)?;
    AnalysisThreadPayload::from_or_fail(eval, "registerAction")?.push_pending(
        PendingAction::BuildInfoFileWrite {
            build,
            translation_callback,
        },
    );
    Ok(StarlarkFile::new(build_info_file))
}

// -------------------------------------------------------------------------------------------
// Finishing pending actions after freezing

/// Builds the actions registered while the rule implementation ran and registers them with the
/// rule context, in registration order (Java registered each one when it was created).
/// `keep_alive` is the frozen keep-alive list (the frozen module's extra value).
pub(crate) fn finish_pending_actions(
    pending: Vec<PendingAction>,
    keep_alive: &OwnedFrozenValue,
    rule_context: &mut RuleContext,
) {
    for action in pending {
        let action: ActionRef = match action {
            PendingAction::Built(action) => action,
            PendingAction::Spawn {
                mut builder,
                command_line,
                resource_set_callback,
            } => {
                // PORT: pending in blaze_exec (execution owner): `new_action_spawn` must pass
                // `SpawnAction::get_resource_set_builder()` to the spawn (Java
                // `BaseSpawn.getLocalResources` -> `buildLocalResources(OS.getCurrent(),
                // inputsSize, ...)`) and turn a `ResourceSetBuildError` into the action's
                // `UserExecException`. Until it does, the spawn books the default resource set
                // (250 MB, 1 CPU); failing loudly there would stop every C++ link, since rules_cc
                // passes a callback on each one.
                if let Some((index, mnemonic, semantics)) = resource_set_callback {
                    builder.set_resources_builder(Arc::new(StarlarkActionResourceSetBuilder {
                        fn_: FrozenAnalysisKeepAlive::get_owned(keep_alive, index),
                        mnemonic,
                        semantics,
                    }));
                }
                for part in command_line {
                    match part {
                        CommandLinePart::Strings(strings) => {
                            builder.add_command_line(command_line::of(strings));
                        }
                        CommandLinePart::Args(index) => {
                            let args = FrozenAnalysisKeepAlive::get_owned(keep_alive, index);
                            // Java: builder.addCommandLine(args.build(repoMappingSupplier),
                            // args.getParamFileInfo())
                            let built = build_args(&args, &mut || {
                                rule_context
                                    .get_analysis_environment()
                                    .get_main_repo_mapping()
                                    .expect("main repository mapping")
                                    .clone()
                            })
                            .expect("frozen Args");
                            builder.add_command_line_with_param_file(
                                built.command_line,
                                built.param_file_info,
                            );
                        }
                    }
                }
                // Java: builder.build(ruleContext) =
                // build(ruleContext.getActionOwner(execGroup), ruleContext.getConfiguration())
                let owner = rule_context
                    .get_action_owner_for(builder.get_exec_group())
                    .expect("NullPointerException: no action owner for the exec group");
                let configuration = rule_context.get_spawn_action_configuration();
                Arc::new(builder.build(owner, &*configuration))
            }
            PendingAction::ParameterFileWrite {
                owner,
                output,
                args,
                is_executable,
                mnemonic,
                unmodified_execution_requirements,
            } => {
                let args = FrozenAnalysisKeepAlive::get_owned(keep_alive, args);
                let built = build_args(&args, &mut || {
                    rule_context
                        .get_analysis_environment()
                        .get_main_repo_mapping()
                        .expect("main repository mapping")
                        .clone()
                })
                .expect("frozen Args");
                let configuration = rule_context.get_spawn_action_configuration();
                let execution_info = configuration
                    .modified_execution_info(&unmodified_execution_requirements, &mnemonic);
                Arc::new(ParameterFileWriteAction::with_inputs(
                    owner,
                    NestedSetBuilder::wrap(Order::STABLE_ORDER, built.directory_artifacts),
                    output,
                    built.command_line,
                    built.parameter_file_type,
                    is_executable,
                    &mnemonic,
                    &execution_info,
                    configuration.get_output_paths_mode(),
                ))
            }
            PendingAction::TemplateExpansion {
                owner,
                template,
                output,
                mut substitutions,
                computed_substitutions,
                executable,
            } => {
                let dict = FrozenAnalysisKeepAlive::get_owned(keep_alive, computed_substitutions);
                let computed = get_all(&dict).expect("frozen TemplateDict");
                for substitution in computed {
                    if substitutions
                        .iter()
                        .any(|s| s.get_key() == substitution.get_key())
                    {
                        // Java: ImmutableMap.Builder.buildOrThrow throws IllegalArgumentException
                        // ("Multiple entries with same key: ...") when expand_template runs.
                        blaze_util::unported!(
                            "StarlarkActionFactory.expandTemplate",
                            "duplicate substitution key {} from computed_substitutions",
                            substitution.get_key()
                        );
                    }
                    substitutions.push(substitution);
                }
                Arc::new(TemplateExpansionAction::for_template_artifact(
                    owner,
                    template,
                    output,
                    substitutions,
                    executable,
                ))
            }
            PendingAction::BuildInfoFileWrite {
                build,
                translation_callback,
            } => build(FrozenAnalysisKeepAlive::get_owned(
                keep_alive,
                translation_callback,
            )),
        };
        rule_context.register_action(action);
    }
}

/// The action owner the factory's rule context gives for actions of the default exec group
/// (Java `ruleContext.getActionOwner()`), for natives that build actions themselves.
pub fn get_action_owner(handle: RuleContextHandle) -> ActionOwner {
    handle.borrow_mut().get_action_owner()
}

#[cfg(test)]
mod resource_set_builder_tests {
    //! Ports of StarlarkRuleContextTest.testCreateStarlarkActionArgumentsWithResourceSet_* at the
    //! level they assert on: `action.getResourceSetOrBuilder().buildResourceSet(os, n)` of the
    //! `StarlarkActionResourceSetBuilder` the factory installs (the factory part needs
    //! BuildViewTestCase; see java_tests.rs). Expected values are the Java tests'.
    use super::*;
    use blaze_actions::analysis::actions::spawn_action::ResourceSetBuilder;
    use blaze_starlark::eval;
    use blaze_starlark::loader::LoadMap;
    use blaze_starlark::{FileKind, Phase};

    fn builder(src: &str) -> StarlarkActionResourceSetBuilder {
        let src = src.to_owned();
        let semantics = Arc::new(StarlarkSemantics::default());
        let module = eval::run_on_evaluation_thread(move || {
            let ctx = blaze_starlark::ThreadContext::new(
                Phase::BzlLoad,
                semantics,
                Arc::new(ResourceSetCallbackEvents),
            );
            let globals = blaze_starlark::env::NativeRegistry::with_blaze_starlark_builtins()
                .build(FileKind::Bzl, &StarlarkSemantics::default());
            eval::compile(FileKind::Bzl, "test.bzl", src)
                .and_then(|c| {
                    eval::exec_module(c, &globals, &LoadMap::new(), &ctx, &Default::default())
                })
                .expect("module")
        });
        StarlarkActionResourceSetBuilder {
            fn_: module.get("get_resources").expect("get_resources"),
            mnemonic: "Action".to_owned(),
            semantics: Arc::new(StarlarkSemantics::default()),
        }
    }

    fn build(b: &StarlarkActionResourceSetBuilder, os: OS, n: i32) -> Result<ResourceSet, String> {
        eval::run_on_evaluation_thread(|| b.build_resource_set(os, n)).map_err(|e| e.message)
    }

    /// Java `ResourceSet.create(memoryMb, cpu, localTestCount)`.
    fn create(memory_mb: f64, cpu_usage: f64, local_test_count: i32) -> ResourceSet {
        ResourceSet {
            memory_mb,
            cpu_usage,
            extra_resource_usage: IndexMap::new(),
            local_test_count,
        }
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_success() {
        let b = builder(
            "def get_resources(os, inputs_size):\n  if os == \"osx\":\n    return {\"cpu\": 2., \"memory\": 350. + inputs_size * 20, \"local_test\": 2.}\n  return {\"cpu\": 1., \"memory\": 350. + inputs_size * 10, \"local_test\": 0.}\n",
        );
        assert_eq!(build(&b, OS::Linux, 2), Ok(create(370.0, 1.0, 0)));
        assert_eq!(build(&b, OS::Darwin, 2), Ok(create(390.0, 2.0, 2)));
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_illegal_resource() {
        let b = builder(
            "def get_resources(os, inputs_size):\n  return {\"cpu\": 2., \"memory\": 350., \"local_test\": 2., \"gpu\": 1.}\n",
        );
        let e = build(&b, OS::Linux, 2).unwrap_err();
        assert!(e.contains("Illegal resource keys: (gpu)"), "{e}");
        assert!(
            e.starts_with("Could not build resources for Action. "),
            "{e}"
        );
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_default_value() {
        let b = builder(
            "def get_resources(os, inputs_size):\n  return {\"cpu\": 2., \"local_test\": 2.}\n",
        );
        assert_eq!(build(&b, OS::Linux, 2), Ok(create(250.0, 2.0, 2)));
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_int_dict() {
        let b = builder(
            "def get_resources(os, inputs_size):\n  return {\"cpu\": 1, \"memory\": 2, \"local_test\": 3}\n",
        );
        assert_eq!(build(&b, OS::Linux, 0), Ok(create(2.0, 1.0, 3)));
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_not_dict() {
        let b = builder("def get_resources(os, inputs_size):\n  return \"keks\"\n");
        let e = build(&b, OS::Linux, 2).unwrap_err();
        assert!(
            e.contains("got string for 'resource_set', want dict"),
            "{e}"
        );
    }

    #[test]
    fn test_create_starlark_action_arguments_with_resource_set_wrong_dict() {
        let b = builder(
            "def get_resources(os, inputs_size):\n  return {\"cpu\": 1, \"memory\": 2, \"local_test\": \"hi\"}\n",
        );
        let e = build(&b, OS::Linux, 2).unwrap_err();
        assert!(
            e.contains("Illegal resource value type for key local_test"),
            "{e}"
        );
    }

    #[test]
    #[ignore = "starlark-rust's own arity error text differs from Java's (accepted core-language text, docs/decisions.md); the failure itself is checked by incorrect_signature_fails"]
    fn test_create_starlark_action_arguments_with_resource_set_incorrect_signature() {
        let b = builder(
            "def get_resources(os):\n  return {\"cpu\": 1, \"memory\": 2, \"local_test\": \"hi\"}\n",
        );
        let e = build(&b, OS::Linux, 2).unwrap_err();
        assert!(
            e.contains("get_resources() accepts no more than 1 positional argument but got 2"),
            "{e}"
        );
    }

    #[test]
    fn incorrect_signature_fails() {
        let b = builder(
            "def get_resources(os):\n  return {\"cpu\": 1, \"memory\": 2, \"local_test\": \"hi\"}\n",
        );
        let e = build(&b, OS::Linux, 2).unwrap_err();
        assert!(
            e.starts_with("Could not build resources for Action. "),
            "{e}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangle_mnemonic_appends_from_starlark() {
        assert_eq!(mangle_mnemonic("CppCompile"), "CppCompileFromStarlark");
    }

    #[test]
    fn closed_error_message() {
        assert_eq!(
            closed_error("actions.run", "//foo:bar").to_string(),
            "cannot access field or method 'actions.run' of rule context for '//foo:bar' outside \
             of its own rule implementation function"
        );
    }

    #[test]
    fn context_repr_rule_and_aspect() {
        assert_eq!(context_repr(false, "//a:b"), "<rule context for //a:b>");
        assert_eq!(context_repr(true, "//a:b"), "<aspect context for //a:b>");
    }

    // Expected texts: the oracle (Bazel 9.3.2) on scripts/w4/scenarios/ctx-errors, targets
    // run_shell_toolchain_list, run_shell_use_default_shell_env_none, run_env_list.
    #[test]
    fn toolchain_param_names_the_method_and_parameter() {
        let list = starlark::values::FrozenValue::new_empty_list().to_value();
        assert_eq!(
            toolchain_param("run_shell", Some(list))
                .unwrap_err()
                .to_string(),
            "in call to run_shell(), parameter 'toolchain' got value of type 'list', want \
             'Label, string, or NoneType'"
        );
        assert!(toolchain_param("run", None).unwrap().is_none());
        assert!(
            toolchain_param("run", Some(Value::new_none()))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn trailing_spawn_params_are_type_checked_in_declaration_order() {
        let none = Value::new_none();
        let list = starlark::values::FrozenValue::new_empty_list().to_value();
        // use_default_shell_env comes first.
        assert_eq!(
            check_trailing_spawn_params("run_shell", none, list, none, none, none, None)
                .unwrap_err()
                .to_string(),
            "in call to run_shell(), parameter 'use_default_shell_env' got value of type \
             'NoneType', want 'bool'"
        );
        assert_eq!(
            check_trailing_spawn_params(
                "run",
                Value::new_bool(false),
                list,
                none,
                none,
                none,
                None
            )
            .unwrap_err()
            .to_string(),
            "in call to run(), parameter 'env' got value of type 'list', want 'dict or NoneType'"
        );
        let (use_default_shell_env, toolchain) = check_trailing_spawn_params(
            "run",
            Value::new_bool(true),
            none,
            none,
            none,
            none,
            Some(none),
        )
        .unwrap();
        assert!(use_default_shell_env);
        assert!(toolchain.is_some());
    }

    // Java: StarlarkActionFactoryApi.run / runShell `@Param` types of outputs, inputs, tools
    // and arguments; texts in the interpreter's format (oracle: ctx-errors, run_outputs_depset
    // and run_inputs_string have the same shape with other value types).
    #[test]
    fn leading_spawn_params_are_type_checked() {
        let none = Value::new_none();
        let list = starlark::values::FrozenValue::new_empty_list().to_value();
        assert_eq!(
            check_outputs_and_inputs_params("run", none, list)
                .unwrap_err()
                .to_string(),
            "in call to run(), parameter 'outputs' got value of type 'NoneType', want 'sequence'"
        );
        assert_eq!(
            check_outputs_and_inputs_params("run", list, none)
                .unwrap_err()
                .to_string(),
            "in call to run(), parameter 'inputs' got value of type 'NoneType', want 'sequence \
             or depset'"
        );
        assert!(check_outputs_and_inputs_params("run_shell", list, list).is_ok());
        assert_eq!(
            check_tools_and_arguments_params("run", Some(none), list)
                .unwrap_err()
                .to_string(),
            "in call to run(), parameter 'tools' got value of type 'NoneType', want 'sequence \
             or depset'"
        );
        assert!(check_tools_and_arguments_params("run", None, list).is_ok());
    }
}
