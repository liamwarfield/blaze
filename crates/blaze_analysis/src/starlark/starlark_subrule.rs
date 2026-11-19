//! The analysis half of `analysis/starlark/StarlarkSubrule.java`: calling a subrule from a rule
//! implementation (`StarlarkSubrule.call`) and the `subrule_ctx` value
//! (`StarlarkSubrule.SubruleContext`) passed to the subrule's implementation.
//!
//! The subrule value itself (`subrule()`, export) is ported in
//! `blaze_packages::starlark_defs::subrule`; its `invoke` dispatches to [`call_subrule`] through
//! the hook the lead installs (see the requests in `mod.rs`).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use allocative::Allocative;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_cmdline::Label;
use blaze_packages::starlark_defs::subrule::{SubruleView, subrule_view};
use blaze_packages::types::Type;
use blaze_starlark::label::LabelValue;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, Heap, NoSerialize, StarlarkValue, Trace, Tracer,
    Value, starlark_value,
};

use super::fragment_collection::FragmentCollection;
use super::starlark_action_factory::files_to_run_provider_to_value;
use super::starlark_action_factory::{
    StarlarkActionContext, StarlarkActionFactory, SubruleActionState,
};
use super::starlark_rule_context::{StarlarkRuleContext, starlark_rule_context_from_value};
use super::starlark_toolchain_context::StarlarkToolchainContext;
use super::thread_payload::{AnalysisThreadPayload, get_starlark_rule_context};
use super::util::errorf;
use crate::configuredtargets::target_to_starlark;
use crate::providers::file_provider::FilesToRunProvider;

/// The identity of an exported subrule (Java `StarlarkSubrule.equals`: extension label and
/// exported name).
fn identity(view: &SubruleView) -> Option<(Label, String)> {
    view.exported.clone()
}

/// The subrules declared by the rule (or aspect) whose context is `ctx` (Java
/// `StarlarkRuleContext.getSubrules()`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:301 StarlarkRuleContext.getSubrules
fn rule_subrules(ctx: &StarlarkRuleContext) -> Vec<(Label, String)> {
    if ctx.is_for_aspect() {
        blaze_util::unported!("StarlarkRuleContext.getSubrules(aspect)");
    }
    ctx.get_rule_class_under_evaluation()
        .get_subrules()
        .iter()
        .filter_map(|s| subrule_view(s.value().to_value()).and_then(|v| identity(&v)))
        .collect()
}

/// `StarlarkSubrule.call(thread, args, kwargs)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:135 StarlarkSubrule.call
pub fn call_subrule<'v>(
    subrule: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let view = subrule_view(subrule).expect("a subrule value");
    // Java: checkExported()
    let Some(this_identity) = identity(&view) else {
        return Err(errorf("Invalid subrule hasn't been exported by a bzl file"));
    };
    let name = view.name.clone();
    // Java: BazelRuleAnalysisThreadContext.fromOrFail(thread, getName()).getRuleContext()
    //           .getStarlarkRuleContext()
    AnalysisThreadPayload::from_or_fail(eval, &name)?;
    let ctx_value = get_starlark_rule_context(eval).expect("the rule's ctx");
    let rule_context = starlark_rule_context_from_value(ctx_value).expect("live ctx");

    let caller_subrule_context = rule_context.get_locked_for_subrule();
    if caller_subrule_context != 0 {
        let caller = active_subrule(eval, caller_subrule_context);
        if !caller.declared_subrules.contains(&this_identity) {
            return Err(errorf(format!(
                "subrule {} must declare {} in 'subrules'",
                caller.state().name,
                name
            )));
        }
    } else if !rule_subrules(rule_context).contains(&this_identity) {
        // Java: getUndeclaredSubruleError(ruleContext)
        return Err(errorf(format!(
            "rule '{}' must declare '{}' in 'subrules'",
            rule_context.get_rule_class_under_evaluation().get_name(),
            name
        )));
    }

    let heap = eval.heap();
    let kwargs: Vec<(String, Value<'v>)> = args
        .names_map()?
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();
    let mut runfiles_from_deps: Vec<Arc<FilesToRunProvider>> = Vec::new();
    let mut named_args: Vec<(String, Value<'v>)> = kwargs.clone();
    let handle = rule_context.get_rule_context();
    for (attr_name, rule_attr_name) in &view.attributes {
        // TODO: b/293304174 - maybe permit overriding?
        if kwargs.iter().any(|(k, _)| k == attr_name) {
            return Err(errorf(format!(
                "got invalid named argument: '{attr_name}' is an implicit dependency and cannot \
                 be overridden"
            )));
        }
        let rc = handle.borrow();
        // Java: getAttributeByName(ruleContext, attr.ruleAttrName)
        let attribute = rc
            .get_rule()
            .get_rule_class_object()
            .get_attribute_provider()
            .get_attribute_by_name(
                rule_attr_name
                    .as_deref()
                    .expect("an exported subrule's attributes have rule attribute names"),
            )
            .clone();
        let prerequisites = rc.get_rule_prerequisites_collection();
        // We need to use the underlying RuleContext because the subrule attributes are hidden
        // from the rule ctx.attr
        let value: Option<Value<'v>> = if attribute.is_executable() {
            let runfiles = prerequisites.get_executable_prerequisite(attribute.get_name());
            match runfiles {
                Some(runfiles) => {
                    runfiles_from_deps.push(runfiles.clone());
                    Some(files_to_run_provider_to_value(heap, runfiles))
                }
                None => None,
            }
        } else if attribute.get_type() == Type::LabelList {
            let targets = prerequisites.get_prerequisites(attribute.get_name());
            Some(
                heap.alloc(AllocList(
                    targets
                        .iter()
                        .map(|t| target_to_starlark(t, heap, rc.get_starlark_semantics())),
                )),
            )
        } else if attribute.get_type() == Type::Label {
            if attribute.is_single_artifact() {
                prerequisites
                    .get_prerequisite_artifact(attribute.get_name())
                    .map(|a| heap.alloc(StarlarkFile::new(a)))
            } else {
                prerequisites
                    .get_prerequisites(attribute.get_name())
                    .first()
                    .map(|t| target_to_starlark(t, heap, rc.get_starlark_semantics()))
            }
        } else {
            // this should never happen, we've already validated the type while evaluating the
            // subrule
            panic!("IllegalStateException: unexpected attribute type");
        };
        named_args.push((attr_name.clone(), value.unwrap_or_else(Value::new_none)));
    }

    let id = rule_context
        .get_action_context()
        .shared
        .next_subrule_id
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let declared_subrules: Vec<(Label, String)> = view
        .subrules
        .iter()
        .filter_map(|s| subrule_view(*s).and_then(|v| identity(&v)))
        .collect();
    let state = Arc::new(SubruleActionState {
        id,
        name: name.clone(),
        requested_toolchains: view
            .toolchains
            .iter()
            .map(|t| t.toolchain_type.clone())
            .collect(),
        runfiles_from_deps,
        alive: AtomicBool::new(true),
    });
    let action_context = StarlarkActionContext {
        subrule: Some(state.clone()),
        ..rule_context.get_action_context().clone()
    };
    let fragments = heap.alloc(FragmentCollection::for_subrule(
        &handle.borrow(),
        &name,
        &view.fragments,
    ));
    let actions = heap.alloc(StarlarkActionFactory::new(action_context.clone()));
    let subrule_context = heap.alloc(SubruleContext {
        context: action_context,
        actions,
        fragments,
        declared_subrules,
    });
    AnalysisThreadPayload::from_or_fail(eval, &name)?
        .subrule_contexts
        .push((id, subrule_context_index(eval, subrule_context)?));

    let mut positionals: Vec<Value<'v>> = vec![subrule_context];
    positionals.extend(args.positions(heap)?);
    let named: Vec<(&str, Value<'v>)> = named_args.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    rule_context.set_locked_for_subrule(id);
    let result = eval.eval_function(view.implementation, &positionals, &named);
    // Java: finally { subruleContext.nullify(); ruleContext.setLockedForSubrule(caller); }
    state.alive.store(false, Ordering::Relaxed);
    // callerSubruleContext may be null if this subrule was called from the rule itself, but in
    // that case null is exactly what we want to set here
    rule_context.set_locked_for_subrule(caller_subrule_context);
    result
}

/// Keeps the subrule context alive for lookups by id while it is active.
fn subrule_context_index<'v>(
    eval: &Evaluator<'v, '_, '_>,
    v: Value<'v>,
) -> starlark::Result<usize> {
    super::thread_payload::keep_alive_push(eval, v)
}

/// The active subrule context with the given id.
fn active_subrule<'v>(eval: &Evaluator<'v, '_, '_>, id: u64) -> &'v SubruleContext<'v> {
    let index = {
        let cx = blaze_starlark::ThreadContext::get(eval).expect("thread context");
        let payload = cx
            .payload::<AnalysisThreadPayload>()
            .expect("analysis payload");
        payload
            .subrule_contexts
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, index)| *index)
            .expect("active subrule context")
    };
    super::thread_payload::keep_alive_get(eval, index)
        .and_then(|v| v.downcast_ref::<SubruleContext>())
        .expect("subrule context")
}

/// The context object passed to the implementation function of a subrule (type
/// `subrule_ctx`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:329 StarlarkSubrule.SubruleContext
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct SubruleContext<'v> {
    #[allocative(skip)]
    context: StarlarkActionContext,
    actions: Value<'v>,
    fragments: Value<'v>,
    /// Java `subrule.getDeclaredSubrules()` (identities).
    #[allocative(skip)]
    declared_subrules: Vec<(Label, String)>,
}

/// The rule context behind a live `StarlarkActionContext` value: a rule `ctx` or a subrule
/// context (Java `StarlarkActionContext.getRuleContext()`), for natives whose parameter is typed
/// `StarlarkActionContext`. `None` for any other value.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:1298 StarlarkActionContext.getRuleContext
pub fn starlark_action_context_rule_context(
    v: Value,
) -> Option<super::thread_payload::RuleContextHandle> {
    if let Some(c) = super::starlark_rule_context::starlark_rule_context_from_value(v) {
        return Some(c.get_rule_context());
    }
    v.downcast_ref::<SubruleContext>().map(|c| c.context.handle)
}

impl<'v> SubruleContext<'v> {
    fn state(&self) -> &SubruleActionState {
        self.context.subrule.as_ref().expect("subrule state")
    }
}

/// A subrule context after its implementation returned.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ClosedSubruleContext {
    repr: String,
}

impl fmt::Debug for SubruleContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SubruleContext")
    }
}

impl fmt::Display for SubruleContext<'_> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:494 SubruleContext.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.context.repr())
    }
}

impl fmt::Display for ClosedSubruleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.repr)
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for SubruleContext<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.actions.trace(tracer);
        self.fragments.trace(tracer);
    }
}

impl<'v> Freeze for SubruleContext<'v> {
    type Frozen = ClosedSubruleContext;

    fn freeze(self, _freezer: &Freezer) -> FreezeResult<ClosedSubruleContext> {
        Ok(ClosedSubruleContext {
            repr: format!("<{} context>", self.state().name),
        })
    }
}

impl<'v> AllocValue<'v> for SubruleContext<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "subrule_ctx")]
impl<'v> StarlarkValue<'v> for SubruleContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("subrule_ctx", subrule_ctx_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

#[starlark_value(type = "subrule_ctx")]
impl<'v> StarlarkValue<'v> for ClosedSubruleContext {
    type Canonical = SubruleContext<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("subrule_ctx", subrule_ctx_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.repr);
    }
}

/// The live subrule context behind `this`, after `checkMutable(attr_name)`.
fn live<'v>(this: Value<'v>, attr_name: &str) -> starlark::Result<&'v SubruleContext<'v>> {
    match this.downcast_ref::<SubruleContext<'v>>() {
        Some(c) => {
            c.context.check_mutable(attr_name)?;
            Ok(c)
        }
        None => Err(errorf(format!(
            "cannot access field or method '{attr_name}' of subrule context outside of its own \
             implementation function"
        ))),
    }
}

#[starlark_module]
fn subrule_ctx_methods(builder: &mut MethodsBuilder) {
    /// The label of the target currently being analyzed
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:359 SubruleContext.getLabel
    #[starlark(attribute)]
    fn label<'v>(this: Value<'v>) -> starlark::Result<LabelValue> {
        let c = live(this, "label")?;
        // we use the underlying RuleContext to bypass the mutability check in
        // StarlarkRuleContext.getLabel() since it's locked
        Ok(LabelValue(c.context.handle.borrow().get_label().clone()))
    }

    /// Contains methods for declaring output files and the actions that produce them
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:372 SubruleContext.actions
    #[starlark(attribute)]
    fn actions<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "actions")?.actions)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:381 SubruleContext.toolchains
    #[starlark(attribute)]
    fn toolchains<'v>(this: Value<'v>) -> starlark::Result<StarlarkToolchainContext> {
        let c = live(this, "toolchains")?;
        let rc = c.context.handle.borrow();
        let Some(toolchain_context) = rc.get_toolchain_context() else {
            return Ok(StarlarkToolchainContext::NotValid);
        };
        if !rc.use_auto_exec_groups() {
            return Err(errorf(
                "subrules using toolchains must enable automatic exec-groups. For more info, see \
                 https://bazel.build/extending/auto-exec-groups#migration-aegs",
            ));
        }
        // Java: getAutomaticExecGroupLabels()
        let requested = &c.state().requested_toolchains;
        let labels: IndexSet<Label> =
            StarlarkRuleContext::get_requested_toolchain_type_labels_from_auto_exec_groups(&rc)
                .into_iter()
                .filter(|l| requested.contains(l))
                .collect();
        Ok(StarlarkToolchainContext::create(
            toolchain_context.target_description().to_owned(),
            &|l| rc.get_toolchain_info(l),
            labels,
        ))
    }

    /// Allows access to configuration fragments in target configuration.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:409 SubruleContext.getFragmentCollection
    #[starlark(attribute)]
    fn fragments<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "fragments")?.fragments)
    }
}
