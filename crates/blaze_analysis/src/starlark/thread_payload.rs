//! The analysis-phase `blaze_starlark::ThreadContext` payload (Java
//! `analysis/BazelRuleAnalysisThreadContext.java`), plus the machinery that lets actions created
//! during a rule implementation be finished after the rule's module is frozen
//! (crates/blaze_analysis/DESIGN.md, "Actions created from Starlark are finished after
//! freezing").
//!
//! - [`AnalysisThreadPayload`] gives natives (`cc_internal`, `py_builtins`, providers) the
//!   current `RuleContext` (Java `BazelRuleAnalysisThreadContext.fromOrFail(thread,
//!   what).getRuleContext()`), and the `ctx` value (Java
//!   `ruleContext.getStarlarkRuleContext()`).
//! - [`register_action`] is Java's `ruleContext.registerAction(action)` for native code: every
//!   action created while the implementation runs goes into one ordered list
//!   ([`PendingAction`]), so registration order equals Java's `registerAction` order.
//! - Starlark values that pending actions need after freezing (copies of `Args` and
//!   `TemplateDict` objects as they were when the action was created, translation functions)
//!   are kept in an [`AnalysisKeepAlive`] list, which is the module's extra value (a GC root,
//!   and frozen with the module).

use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::ptr::NonNull;

use allocative::Allocative;
use blaze_actions::ActionRef;
use blaze_actions::Artifact;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::analysis::actions::spawn_action::SpawnActionBuilder;
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_starlark::ThreadContext;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::eval::Evaluator;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, OwnedFrozenValue,
    StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use crate::rule::RuleContext;

use super::util::errorf;

// -------------------------------------------------------------------------------------------
// RuleContextHandle

/// A handle on the `RuleContext` whose implementation function is running.
///
/// Java's `StarlarkRuleContext`, `StarlarkActionFactory`, `ctx.outputs` and the thread context
/// all hold the `RuleContext` object. Here the rule context is owned by the caller of
/// [`super::starlark_rule_configured_target_util::build_rule`], which lends it for the
/// evaluation through a `RefCell` (so overlapping borrows are detected at run time).
///
/// The handle is only valid while that evaluation runs: values holding it are never frozen
/// with it (`ctx`, `ctx.actions` and `ctx.outputs` freeze into closed values without the
/// handle, which is Java's `StarlarkRuleContext.close()`), and the thread payload is taken back
/// before `build_rule` returns. Native code must not keep a handle in frozen values.
#[derive(Clone, Copy)]
pub struct RuleContextHandle {
    cell: NonNull<RefCell<&'static mut RuleContext>>,
}

// SAFETY: the handle is used only on the thread running the rule's evaluation, while the
// borrowed rule context is alive (see the type docs). Starlark requires unfrozen values (which
// hold the handle) to be `Send`.
unsafe impl Send for RuleContextHandle {}
// SAFETY: as above; shared access goes through the `RefCell`, on one thread.
unsafe impl Sync for RuleContextHandle {}

impl RuleContextHandle {
    /// Creates a handle on `cell`.
    ///
    /// # Safety
    ///
    /// `cell` must outlive every use of the returned handle (and its copies).
    pub(crate) unsafe fn new(cell: &RefCell<&mut RuleContext>) -> RuleContextHandle {
        let ptr =
            cell as *const RefCell<&mut RuleContext> as *mut RefCell<&'static mut RuleContext>;
        RuleContextHandle {
            // SAFETY: `ptr` comes from a reference, so it is not null.
            cell: unsafe { NonNull::new_unchecked(ptr) },
        }
    }

    fn cell(&self) -> &RefCell<&'static mut RuleContext> {
        // SAFETY: valid while the evaluation runs (type docs).
        unsafe { self.cell.as_ref() }
    }

    /// Borrows the rule context. Panics if it is mutably borrowed (a bug in a native).
    pub fn borrow(&self) -> Ref<'_, RuleContext> {
        Ref::map(self.cell().borrow(), |r| &**r)
    }

    /// Mutably borrows the rule context. Panics if it is borrowed (a bug in a native).
    pub fn borrow_mut(&self) -> RefMut<'_, RuleContext> {
        RefMut::map(self.cell().borrow_mut(), |r| &mut **r)
    }
}

impl fmt::Debug for RuleContextHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RuleContextHandle")
    }
}

// -------------------------------------------------------------------------------------------
// Pending actions

/// One part of a spawn action's command line (Java `StarlarkActionFactory.buildCommandLine`):
/// a run of plain strings (`CommandLine.of(stringArgs)`) or an `Args` object (with its param
/// file info) as it was when the action was created, referenced by the index of that copy in
/// the [`AnalysisKeepAlive`] list.
#[derive(Debug)]
pub(crate) enum CommandLinePart {
    Strings(Vec<String>),
    Args(usize),
}

/// An action registered while the rule implementation runs, in registration order.
pub(crate) enum PendingAction {
    /// An action built at registration time (native code, and Starlark actions that hold no
    /// Starlark values).
    Built(ActionRef),
    /// `ctx.actions.run` / `run_shell` (Java `registerAction(builder.build(ruleContext))`): the
    /// builder with everything but the command lines, which need frozen `Args`.
    Spawn {
        builder: Box<SpawnActionBuilder>,
        command_line: Vec<CommandLinePart>,
        /// A `resource_set` callback (Java `StarlarkActionResourceSetBuilder`): the callable's
        /// index in the keep-alive list, the action's mnemonic and the rule's semantics.
        resource_set_callback: Option<(
            usize,
            String,
            std::sync::Arc<blaze_starlark::StarlarkSemantics>,
        )>,
    },
    /// `ctx.actions.write` with an `Args` content (Java `new ParameterFileWriteAction(...)`).
    ParameterFileWrite {
        owner: ActionOwner,
        output: Artifact,
        /// The index, in the keep-alive list, of the copy of the `Args` taken at the call.
        args: usize,
        is_executable: bool,
        mnemonic: String,
        /// `TargetUtils.getFilteredExecutionInfo(...)` (before `modifiedExecutionInfo`).
        unmodified_execution_requirements: IndexMap<String, String>,
    },
    /// `ctx.actions.expand_template` with `computed_substitutions` (a `TemplateDict`).
    TemplateExpansion {
        owner: ActionOwner,
        template: Artifact,
        output: Artifact,
        /// The plain substitutions, in dict order.
        substitutions: Vec<Substitution>,
        /// The index, in the keep-alive list, of the copy of the `TemplateDict` taken at the
        /// call.
        computed_substitutions: usize,
        executable: bool,
    },
    /// `ctx.actions.transform_info_file` / `transform_version_file` (Java
    /// `registerAction(new BuildInfoFileWriteAction(...))`): builds the action from the frozen
    /// translation function.
    BuildInfoFileWrite {
        build: super::starlark_action_factory::BuildInfoFileWriteActionBuilder,
        /// The translation function's index in the keep-alive list.
        translation_callback: usize,
    },
}

impl fmt::Debug for PendingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PendingAction::Built(a) => write!(f, "Built({a:?})"),
            PendingAction::Spawn { command_line, .. } => {
                write!(f, "Spawn({command_line:?})")
            }
            PendingAction::ParameterFileWrite { output, .. } => {
                write!(f, "ParameterFileWrite({output:?})")
            }
            PendingAction::TemplateExpansion { output, .. } => {
                write!(f, "TemplateExpansion({output:?})")
            }
            PendingAction::BuildInfoFileWrite {
                translation_callback,
                ..
            } => write!(f, "BuildInfoFileWrite({translation_callback})"),
        }
    }
}

// -------------------------------------------------------------------------------------------
// AnalysisThreadPayload

/// The payload of an analysis `ThreadContext` (Java `BazelRuleAnalysisThreadContext`).
// Java: com/google/devtools/build/lib/analysis/BazelRuleAnalysisThreadContext.java:28 BazelRuleAnalysisThreadContext
pub struct AnalysisThreadPayload {
    rule_context: RuleContextHandle,
    pub(crate) pending_actions: Vec<PendingAction>,
    /// Index of the `ctx` value in the keep-alive list (Java
    /// `ruleContext.getStarlarkRuleContext()`), once created.
    pub(crate) starlark_rule_context: Option<usize>,
    /// Active subrule contexts: (id, index in the keep-alive list).
    pub(crate) subrule_contexts: Vec<(u64, usize)>,
}

impl AnalysisThreadPayload {
    // Java: com/google/devtools/build/lib/analysis/BazelRuleAnalysisThreadContext.java:37 BazelRuleAnalysisThreadContext.<init>
    pub(crate) fn new(rule_context: RuleContextHandle) -> AnalysisThreadPayload {
        AnalysisThreadPayload {
            rule_context,
            pending_actions: Vec::new(),
            starlark_rule_context: None,
            subrule_contexts: Vec::new(),
        }
    }

    /// Java `BazelRuleAnalysisThreadContext.fromOrFail(thread, what)`: the payload of the
    /// running analysis thread, or "`what` can only be called from a rule or aspect
    /// implementation". Drop the returned borrow before calling back into Starlark.
    // Java: com/google/devtools/build/lib/analysis/BazelRuleAnalysisThreadContext.java:62 BazelRuleAnalysisThreadContext.fromOrFail
    pub fn from_or_fail<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> starlark::Result<RefMut<'a, AnalysisThreadPayload>> {
        let fail = || {
            errorf(format!(
                "{what} can only be called from a rule or aspect implementation"
            ))
        };
        let cx = ThreadContext::get(eval).map_err(|_| fail())?;
        cx.payload::<AnalysisThreadPayload>().ok_or_else(fail)
    }

    /// Java `fromOrFail(...).getRuleContext()`: the rule context handle.
    // Java: com/google/devtools/build/lib/analysis/BazelRuleAnalysisThreadContext.java:48 BazelRuleAnalysisThreadContext.getRuleContext
    pub fn get_rule_context(&self) -> RuleContextHandle {
        self.rule_context
    }

    /// Java `getAnalysisRuleLabel()`.
    // Java: com/google/devtools/build/lib/analysis/BazelRuleAnalysisThreadContext.java:43 BazelRuleAnalysisThreadContext.getAnalysisRuleLabel
    pub fn get_analysis_rule_label(&self) -> blaze_cmdline::Label {
        self.rule_context.borrow().get_label().clone()
    }

    /// Appends an action to the ordered registration list.
    pub(crate) fn push_pending(&mut self, action: PendingAction) {
        self.pending_actions.push(action);
    }
}

/// The rule context of the running analysis thread (Java
/// `BazelRuleAnalysisThreadContext.fromOrFail(thread, what).getRuleContext()`).
pub fn rule_context_from_or_fail(
    eval: &Evaluator,
    what: &str,
) -> starlark::Result<RuleContextHandle> {
    Ok(AnalysisThreadPayload::from_or_fail(eval, what)?.get_rule_context())
}

/// Java `ruleContext.registerAction(action)` for native code running inside a rule
/// implementation (for example `cc_internal.create_cc_compile_action`): the action joins the
/// ordered list of the target's actions, after every action registered before it.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:564 RuleContext.registerAction
pub fn register_action(eval: &Evaluator, action: ActionRef) -> starlark::Result<()> {
    AnalysisThreadPayload::from_or_fail(eval, "registerAction")?
        .push_pending(PendingAction::Built(action));
    Ok(())
}

/// The `ctx` value of the running rule implementation (Java
/// `ruleContext.getStarlarkRuleContext()`), if the thread is an analysis thread.
pub fn get_starlark_rule_context<'v>(eval: &Evaluator<'v, '_, '_>) -> Option<Value<'v>> {
    let index = {
        let cx = ThreadContext::get(eval).ok()?;
        let payload = cx.payload::<AnalysisThreadPayload>()?;
        payload.starlark_rule_context?
    };
    keep_alive_get(eval, index)
}

// -------------------------------------------------------------------------------------------
// AnalysisKeepAlive

/// Starlark values that must survive (frozen) the rule implementation's module, in the order
/// they were kept: the module's extra value.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct AnalysisKeepAlive<'v> {
    #[allocative(skip)]
    values: RefCell<Vec<Value<'v>>>,
}

/// The frozen [`AnalysisKeepAlive`].
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenAnalysisKeepAlive {
    #[allocative(skip)]
    values: Vec<FrozenValue>,
}

impl<'v> AnalysisKeepAlive<'v> {
    pub(crate) fn new() -> AnalysisKeepAlive<'v> {
        AnalysisKeepAlive {
            values: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn push(&self, v: Value<'v>) -> usize {
        let mut values = self.values.borrow_mut();
        values.push(v);
        values.len() - 1
    }

    pub(crate) fn get(&self, index: usize) -> Option<Value<'v>> {
        self.values.borrow().get(index).copied()
    }
}

impl FrozenAnalysisKeepAlive {
    /// The kept value at `index`, owned by `owner` (the frozen keep-alive value itself).
    pub(crate) fn get_owned(owner: &OwnedFrozenValue, index: usize) -> OwnedFrozenValue {
        let v = owner
            .value()
            .downcast_ref::<FrozenAnalysisKeepAlive>()
            .expect("frozen keep-alive list")
            .values[index];
        owner.map(|_| v)
    }
}

impl fmt::Debug for AnalysisKeepAlive<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AnalysisKeepAlive")
    }
}

impl fmt::Display for AnalysisKeepAlive<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<analysis keep-alive>")
    }
}

impl fmt::Debug for FrozenAnalysisKeepAlive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrozenAnalysisKeepAlive")
    }
}

impl fmt::Display for FrozenAnalysisKeepAlive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<analysis keep-alive>")
    }
}

// SAFETY: `trace` visits every value held.
unsafe impl<'v> Trace<'v> for AnalysisKeepAlive<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for v in self.values.get_mut() {
            v.trace(tracer);
        }
    }
}

impl<'v> Freeze for AnalysisKeepAlive<'v> {
    type Frozen = FrozenAnalysisKeepAlive;

    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenAnalysisKeepAlive> {
        let mut values = Vec::new();
        for v in self.values.into_inner() {
            values.push(freezer.freeze(v)?);
        }
        Ok(FrozenAnalysisKeepAlive { values })
    }
}

impl<'v> AllocValue<'v> for AnalysisKeepAlive<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "analysis_keep_alive")]
impl<'v> StarlarkValue<'v> for AnalysisKeepAlive<'v> {}

#[starlark_value(type = "analysis_keep_alive")]
impl<'v> StarlarkValue<'v> for FrozenAnalysisKeepAlive {
    type Canonical = AnalysisKeepAlive<'v>;
}

/// Keeps `v` alive (and frozen with the module) for the pending actions; returns its index.
pub(crate) fn keep_alive_push<'v>(
    eval: &Evaluator<'v, '_, '_>,
    v: Value<'v>,
) -> starlark::Result<usize> {
    let keep = eval
        .module()
        .extra_value()
        .and_then(|k| k.downcast_ref::<AnalysisKeepAlive>())
        .ok_or_else(|| errorf("internal error: no analysis keep-alive list"))?;
    Ok(keep.push(v))
}

/// The value kept at `index`.
pub(crate) fn keep_alive_get<'v>(eval: &Evaluator<'v, '_, '_>, index: usize) -> Option<Value<'v>> {
    let extra = eval.module().extra_value()?;
    let keep = extra.downcast_ref::<AnalysisKeepAlive>()?;
    keep.get(index).map(|v| v.to_value())
}
