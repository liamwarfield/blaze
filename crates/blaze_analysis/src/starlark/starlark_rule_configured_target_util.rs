//! Port of `analysis/starlark/StarlarkRuleConfiguredTargetUtil.java`: evaluates a Starlark rule's
//! implementation function and turns what it returns into a configured target; plus the Starlark
//! branch of `ConfiguredTargetFactory.createRule` that drives it ([`build_rule`]).
//!
//! The implementation runs in a fresh `Module` (crates/blaze_analysis/DESIGN.md, "Rule
//! implementations run per configured target in a fresh Module"): [`build_rule`] creates `ctx`,
//! calls the frozen implementation, closes the context, freezes the module (so the returned
//! providers are frozen values that keep the module's heap alive), builds the pending actions in
//! registration order, and then validates the providers and builds the target exactly like
//! Java's `createTarget`.

use std::cell::RefCell;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles::{Runfiles, RunfilesBuilder};
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::build::package_factory::DebugPrintHandler;
use blaze_packages::rule_class::rule_class::RuleClass;
use blaze_packages::rule_class::{AdvertisedProviderSet, ProviderKey};
use blaze_packages::starlark_defs::builtin_providers::{
    MATERIALIZED_DEPS_INFO, RUN_ENVIRONMENT_INFO, STRUCT,
};
use blaze_packages::target::target_utils::is_test_rule_name;
use blaze_packages::types::{AttrValue, Type};
use blaze_starlark::depset::Depset;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::{EvalError, Phase, ThreadContext};
use indexmap::IndexMap;
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{FrozenValue, OwnedFrozenValue, Value, ValueLike};

use super::starlark_action_factory::finish_pending_actions;
use super::starlark_rule_context::StarlarkRuleContext;
use super::thread_payload::{AnalysisKeepAlive, AnalysisThreadPayload, RuleContextHandle};
use super::util::{
    depset_cast_artifacts, depset_get_set_artifacts, errorf, is_iterable, stable_order_set,
};
use crate::configured_rule_class_provider::CreateConfiguredTargetError;
use crate::configured_target::ConfiguredTarget;
use crate::info::{downcast_info, info_provider_key, is_info};
use crate::providers::default_info::{DefaultInfo, FrozenDefaultInfo};
use crate::providers::file_provider::RunfilesProvider;
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::providers::run_environment_info::RunEnvironmentInfo;
use crate::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::rule::{RuleContext, RuleErrorException};

/// What the Starlark branch of `ConfiguredTargetFactory.createRule` produces.
pub struct StarlarkRuleOutcome {
    /// The configured target; `None` where Java returns null: the errors were reported on the
    /// rule context and the caller returns `erroredConfiguredTarget(ruleContext,
    /// requiredConfigFragmentsProvider)`.
    pub target: Option<ConfiguredTarget>,
    /// Java `ruleContext.getRequiredConfigFragments()`, computed before the rule context was
    /// closed.
    pub required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
}

/// What the evaluation leaves behind once the module is frozen.
struct Evaluated {
    /// The frozen module (its extra value is the frozen keep-alive list).
    frozen: Option<FrozenModule>,
    /// Index of the raw providers in the keep-alive list; `None` where `evalRule` returned null.
    raw_providers: Option<usize>,
    is_default_executable_created: bool,
    required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
}

/// The Starlark branch of `ConfiguredTargetFactory.createRule` (Java: `initStarlarkRuleContext`,
/// `evalRule`, `isDefaultExecutableCreated`, `getRequiredConfigFragments`, `close`, then
/// `createTarget`).
///
/// Must run on a thread with a large native stack (`blaze_starlark::eval::EVAL_STACK_SIZE`),
/// like every Starlark evaluation. `Err(RuleError)` is Java's `RuleErrorException` from
/// `initStarlarkRuleContext` (the caller returns `erroredConfiguredTarget(ruleContext, null)`).
// Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:452 ConfiguredTargetFactory.createRule
pub fn build_rule(
    rule_context: &mut RuleContext,
    rule_class: &Arc<RuleClass>,
) -> Result<StarlarkRuleOutcome, CreateConfiguredTargetError> {
    let advertised_providers = rule_class.get_advertised_providers().clone();
    let thread = analysis_thread_context(rule_context);

    let cell: RefCell<&mut RuleContext> = RefCell::new(rule_context);
    // SAFETY: `cell` outlives the evaluation below, which is the only user of the handle:
    // every value holding it lives on the module's heap (dropped at the end of
    // `with_temp_heap`) or freezes without it, and the payload is taken back before `cell` is
    // dropped.
    let handle = unsafe { RuleContextHandle::new(&cell) };
    let thread = thread.with_payload(AnalysisThreadPayload::new(handle));

    let evaluated = evaluate(&thread, handle, rule_class);

    let payload = thread
        .take_payload::<AnalysisThreadPayload>()
        .expect("the analysis payload is only taken here");
    let rule_context: &mut RuleContext = cell.into_inner();
    let evaluated = evaluated.map_err(CreateConfiguredTargetError::RuleError)?;

    // Java registered each action as it was created; build them now that their Starlark values
    // are frozen, in the same order.
    if let Some(frozen) = &evaluated.frozen {
        let keep_alive = frozen
            .owned_extra_value()
            .expect("the keep-alive list is the module's extra value");
        finish_pending_actions(payload.pending_actions, &keep_alive, rule_context);
    }

    let raw_providers = match (&evaluated.frozen, evaluated.raw_providers) {
        (Some(frozen), Some(index)) => {
            let keep_alive = frozen.owned_extra_value().expect("keep-alive list");
            Some(super::thread_payload::FrozenAnalysisKeepAlive::get_owned(
                &keep_alive,
                index,
            ))
        }
        _ => None,
    };
    let Some(raw_providers) = raw_providers else {
        return Ok(StarlarkRuleOutcome {
            target: None,
            required_config_fragments_provider: evaluated.required_config_fragments_provider,
        });
    };
    // Because ruleContext was closed, rawProviders are now immutable
    // Postprocess providers to create the finished target.
    let target = create_target(
        rule_context,
        rule_class,
        raw_providers,
        &advertised_providers,
        evaluated.is_default_executable_created,
        evaluated.required_config_fragments_provider.clone(),
    )?;
    Ok(StarlarkRuleOutcome {
        target,
        required_config_fragments_provider: evaluated.required_config_fragments_provider,
    })
}

/// The thread of a rule implementation (Java `RuleContext.createStarlarkThread`): analysis
/// phase, the analysis environment's semantics, `print` to its event handler, and the main
/// repository mapping (Java `BazelRuleAnalysisThreadContext` extends `StarlarkThreadContext`).
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:977 RuleContext.createStarlarkThread
pub(crate) fn analysis_thread_context(rule_context: &RuleContext) -> ThreadContext {
    let env = rule_context.get_analysis_environment();
    let semantics = env.get_starlark_semantics().clone();
    let handler: Arc<dyn blaze_events::EventHandler> = env.get_event_handler().clone();
    let events = Arc::new(DebugPrintHandler::new(handler));
    // Java registers no .bzl module on the thread: `Label()` reads the module of the innermost
    // Starlark function from the function itself.
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:174 BazelModuleContext.ofInnermostBzlOrFail
    ThreadContext::new(Phase::Analysis, semantics, events).with_main_repo_mapping(
        env.get_main_repo_mapping()
            .expect("MissingDepException: main repository mapping")
            .clone(),
    )
}

/// Runs the implementation in a fresh module and freezes it.
fn evaluate(
    thread: &ThreadContext,
    handle: RuleContextHandle,
    rule_class: &Arc<RuleClass>,
) -> Result<Evaluated, RuleErrorException> {
    let opts = EvalOptions::default();
    Module::with_temp_heap(|module| {
        let keep_alive = module.heap().alloc(AnalysisKeepAlive::new());
        module.set_extra_value(keep_alive);
        let keep = keep_alive
            .downcast_ref::<AnalysisKeepAlive>()
            .expect("keep-alive list");

        let raw_providers;
        let is_default_executable_created;
        let required_config_fragments_provider;
        // Java: a Depset created by the implementation wraps one NestedSet for as long as it
        // lives, before and after its Mutability is frozen; see rule::depsets.
        // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
        let depset_scope = crate::rule::depsets::DepsetConversionScope::enter();
        {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(thread);
            eval.set_forbid_recursion(!thread.semantics.allow_recursion);
            eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                .expect("fresh evaluator");

            // must be called before any calls to ruleContext.getStarlarkRuleContext()
            // Java: com/google/devtools/build/lib/analysis/RuleContext.java:996 RuleContext.initStarlarkRuleContext
            let ctx = StarlarkRuleContext::create(handle, None, module.heap())?;
            let ctx_value = module.heap().alloc(ctx);
            let ctx_index = keep.push(ctx_value);
            AnalysisThreadPayload::from_or_fail(&eval, "initStarlarkRuleContext")
                .expect("analysis payload")
                .starlark_rule_context = Some(ctx_index);
            let ctx = ctx_value
                .downcast_ref::<StarlarkRuleContext>()
                .expect("ctx");

            // TODO(bazel-team): maybe merge with RuleConfiguredTargetBuilder?
            let raw = eval_rule(handle, rule_class, ctx_value, &mut eval);
            // TODO(b/268525292): isDefaultExecutableCreated is set to True when
            // ctx.outputs.executable is accessed in the implementation. This fragile mechanism
            // should be revised and removed
            is_default_executable_created = ctx.is_default_executable_created();
            // PORT: R's RuleContext records the variables read through ctx.var itself
            // (`starlark_looked_up_variables`); `ctx.looked_up_variables()` is not needed here.
            required_config_fragments_provider = handle.borrow().get_required_config_fragments();
            // Java: finally { ruleContext.close(); }
            ctx.close();
            raw_providers = raw.map(|v| keep.push(v));
        }
        // PORT: freezing copies the depsets the implementation made; the keep-alive list gives
        // the frozen copy of each one whose NestedSet an action already holds.
        // SAFETY: the depsets converted in the scope are values of `module`'s heap, which is
        // alive and is not garbage collected while a function call (the implementation) runs.
        let converted_depsets: Vec<usize> = unsafe { depset_scope.converted_depsets() }
            .into_iter()
            .map(|depset| keep.push(depset))
            .collect();
        let frozen = match module.freeze() {
            Ok(frozen) => {
                // PORT: keep-alive. rule::depsets memoizes NestedSets by the address of frozen
                // depsets, so a frozen heap must never be freed (and its addresses reused), not
                // even when the target later fails analysis.
                crate::rule::depsets::keep_frozen_heap_alive(frozen.frozen_heap().clone());
                let keep_alive = frozen
                    .owned_extra_value()
                    .expect("the keep-alive list is the module's extra value");
                let frozen_depsets: Vec<OwnedFrozenValue> = converted_depsets
                    .iter()
                    .map(|index| {
                        super::thread_payload::FrozenAnalysisKeepAlive::get_owned(
                            &keep_alive,
                            *index,
                        )
                    })
                    .collect();
                depset_scope.adopt_frozen(&frozen_depsets);
                Some(frozen)
            }
            Err(e) => {
                // PORT: Java's providers are frozen by closing the Mutability, which cannot fail;
                // starlark-rust refuses values that cannot be frozen.
                handle.borrow_mut().rule_error(&format!("\n{e:?}"));
                None
            }
        };
        Ok(Evaluated {
            raw_providers: if frozen.is_some() {
                raw_providers
            } else {
                None
            },
            frozen,
            is_default_executable_created,
            required_config_fragments_provider,
        })
    })
}

/// Evaluates the rule's implementation function and returns what it returns (raw providers).
///
/// If there were errors during the evaluation or the type of the returned object is obviously
/// wrong, it sets ruleErrors on the ruleContext and returns `None`.
///
/// `rule_class` is the rule class for which to evaluate the implementation function. This
/// serves extended rules, where for parent's implementation function needs to be evaluated.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:76 StarlarkRuleConfiguredTargetUtil.evalRule
pub(crate) fn eval_rule<'v>(
    rule_context: RuleContextHandle,
    rule_class: &RuleClass,
    starlark_rule_context: Value<'v>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Option<Value<'v>> {
    // TODO(blaze-team): expect_failure attribute is special for all rule classes, but it should
    // be special only for analysis tests
    let expect_failure = match rule_context
        .borrow()
        .attributes()
        .get("expect_failure", Type::String)
    {
        Some(AttrValue::String(s)) => s,
        _ => String::new(),
    };

    // call rule.implementation(ctx)
    let function = rule_class
        .get_configured_target_function()
        .expect("Starlark rule class without an implementation function")
        .to_owned_frozen_value();
    let function = eval.heap().access_owned_frozen_value(&function);
    let providers_raw = match eval.eval_function(function, &[starlark_rule_context], &[]) {
        Ok(v) => v,
        Err(e) => {
            // An error occurred during the rule.implementation call
            let ex = EvalError::from_starlark(&e);

            // If the error was expected by an analysis test, return None, to produce an empty
            // target.
            if !expect_failure.is_empty() {
                // Java: ex.getMessage().matches(expectFailure) (a java.util.regex match)
                blaze_util::unported!(
                    "StarlarkRuleConfiguredTargetUtil.evalRule(expect_failure)",
                    "regex match of '{}'",
                    expect_failure
                );
            }

            // Emit a single event that spans multiple lines:
            //     ERROR p/BUILD:1:1: in foo_library rule //p:p:
            //     Traceback:
            //        File foo.bzl, line 1, in foo_library_impl:
            //        ...
            rule_context
                .borrow_mut()
                .rule_error(&format!("\n{}", ex.message_with_stack()));
            return None;
        }
    };

    // Errors already reported?
    if rule_context.borrow().has_errors() {
        return None;
    }

    // Wrong result type?
    if !(is_info(providers_raw) || providers_raw.is_none() || is_iterable(providers_raw)) {
        rule_context.borrow_mut().rule_error(&format!(
            "Rule should return a struct or a list, but got {}",
            providers_raw.get_type()
        ));
        return None;
    }

    // Did the Starlark implementation function fail to fail as expected?
    if !expect_failure.is_empty() {
        rule_context
            .borrow_mut()
            .rule_error(&format!("Expected failure not found: {expect_failure}"));
        return None;
    }

    Some(providers_raw)
}

/// The location of the rule class's implementation function (Java
/// `getConfiguredTargetFunction().getLocation()`), used in error messages.
fn impl_location(rule_class: &RuleClass) -> String {
    let _ = rule_class;
    // PORT: starlark-rust does not expose the definition location of a function.
    blaze_util::unported!("StarlarkFunction.getLocation")
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:138 StarlarkRuleConfiguredTargetUtil.checkDeclaredProviders
fn check_declared_providers(
    configured_target: &ConfiguredTarget,
    advertised_providers: &AdvertisedProviderSet,
) -> Result<(), String> {
    for provider_id in advertised_providers.get_starlark_providers() {
        if configured_target.get(provider_id.get_key()).is_none() {
            return Err(format!(
                "rule advertised the '{provider_id}' provider, but this provider was not among \
                 those returned"
            ));
        }
    }
    Ok(())
}

/// Creates a Rule Configured Target from the raw providers returned by the rule's
/// implementation function.
///
/// If there are problems with the raw providers, it sets ruleErrors on the ruleContext and
/// returns `None`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:158 StarlarkRuleConfiguredTargetUtil.createTarget
pub fn create_target(
    context: &RuleContext,
    rule_class: &RuleClass,
    raw_providers: OwnedFrozenValue,
    advertised_providers: &AdvertisedProviderSet,
    is_default_executable_created: bool,
    required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
    let mut builder = RuleConfiguredTargetBuilder::new(context);

    // TODO(adonovan): clean up addProviders' error handling,
    // reporting provider validity errors through ruleError
    // where possible. This allows for multiple events, with independent
    // locations, even for the same root cause.
    // The required change is fiddly due to frequent and nested use of
    // Structure.getField, Sequence.cast, and similar operators.
    if let Err(ex) = add_providers(
        context,
        rule_class,
        &mut builder,
        &raw_providers,
        is_default_executable_created,
    )? {
        // Emit a single event that spans two lines (see infoError).
        // The message typically starts with another location, e.g. of provider creation.
        //     ERROR p/BUILD:1:1: in foo_library rule //p:p:
        //     ...message...
        context.rule_error(&format!("\n{ex}"));
        return Ok(None);
    }

    // This provider is kept out of `addProviders` method, because it's not generated by the
    // Starlark rule and because `addProviders` will be simplified by the legacy providers removal
    // RequiredConfigFragmentsProvider may be removed with removal of Android feature flags.
    if let Some(required_config_fragments_provider) = required_config_fragments_provider {
        builder.add_provider(required_config_fragments_provider);
    }

    // This also throws InterruptedException from a convoluted dependency:
    // TestActionBuilder -> TestTargetExecutionSettings -> CommandLine -> Starlark.
    // PORT: Java catches the IllegalArgumentException `build()` may throw and reports
    // "\n" + implLoc + ": " + message as a rule error; R's builder has no such error.
    let ct = match builder.build() {
        Ok(ct) => ct, // may be null
        Err(RuleConfiguredTargetBuildError::ActionConflict(msg)) => {
            return Err(CreateConfiguredTargetError::ActionConflict(msg));
        }
        Err(RuleConfiguredTargetBuildError::Interrupted) => {
            return Err(CreateConfiguredTargetError::Interrupted);
        }
    };

    if let Some(ct) = &ct {
        // If there was error creating the ConfiguredTarget, no further validation is needed.
        // Null will be returned and the errors thus reported.
        // Check all artifacts have actions. Despite signature, must be done after build().
        let result = validate_artifacts(context)
            // Check all advertised providers were created.
            .and_then(|()| check_declared_providers(ct, advertised_providers));
        if let Err(ex) = result {
            let loc = impl_location(rule_class);
            context.rule_error(&format!("\n{loc}: {ex}"));
            return Ok(None);
        }
    }

    Ok(ct)
}

/// `StarlarkProviderValidationUtil.validateArtifacts(ruleContext)`.
// Java: com/google/devtools/build/lib/analysis/StarlarkProviderValidationUtil.java:25 StarlarkProviderValidationUtil.validateArtifacts
pub(crate) fn validate_artifacts(rule_context: &RuleContext) -> Result<(), String> {
    let env = rule_context.get_analysis_environment();
    let tree_artifacts_conflicting_with_files = env.get_tree_artifacts_conflicting_with_files();
    if !tree_artifacts_conflicting_with_files.is_empty() {
        return Err(format!(
            "The following directories were also declared as files:\n{}",
            artifacts_description(&tree_artifacts_conflicting_with_files)
        ));
    }

    let orphan_artifacts = env.get_orphan_artifacts();
    if !orphan_artifacts.is_empty() {
        return Err(format!(
            "The following files have no generating action:\n{}",
            artifacts_description(&orphan_artifacts)
        ));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/StarlarkProviderValidationUtil.java:44 StarlarkProviderValidationUtil.artifactsDescription
fn artifacts_description<'a>(artifacts: impl IntoIterator<Item = &'a Artifact>) -> String {
    artifacts
        .into_iter()
        .map(|a| a.get_root_relative_path_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `convertToOutputGroupValue(outputGroup, objects)`: a sequence of files becomes a stable-order
/// set; a depset of files is used as is.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:227 StarlarkRuleConfiguredTargetUtil.convertToOutputGroupValue
pub fn convert_to_output_group_value(
    output_group: &str,
    objects: Value,
) -> starlark::Result<NestedSet<Artifact>> {
    // regrettable preemptive allocation of error message
    let what = format!("output group '{output_group}'");
    let elements: Option<Vec<Value>> = if let Some(l) = ListRef::from_value(objects) {
        Some(l.content().to_vec())
    } else if let Some(t) = TupleRef::from_value(objects) {
        Some(t.content().to_vec())
    } else if objects.get_type() == "range" {
        if objects.length()? > 0 {
            return Err(errorf(format!(
                "at index 0 of {what}, got element of type int, want File"
            )));
        }
        Some(Vec::new())
    } else {
        None
    };
    match elements {
        Some(elements) => {
            let mut artifacts = Vec::with_capacity(elements.len());
            for (i, elem) in elements.into_iter().enumerate() {
                match artifact_from_value(elem) {
                    Some(a) => artifacts.push(a),
                    None => {
                        return Err(errorf(format!(
                            "at index {i} of {what}, got element of type {}, want File",
                            elem.get_type()
                        )));
                    }
                }
            }
            Ok(stable_order_set(artifacts))
        }
        None => {
            if Depset::from_value(objects).is_none() {
                return Err(errorf(format!(
                    "for {what}, got {}, want a depset of File",
                    objects.get_type()
                )));
            }
            depset_get_set_artifacts(objects).map_err(|e| errorf(format!("for '{what}', {e}")))
        }
    }
}

/// The fields of a `DefaultInfo` that `parseDefaultProviderFields` reads, and the location
/// `infoError` prefixes messages with.
struct DefaultInfoFields {
    /// `getFiles()` cast to a `NestedSet<Artifact>` (`Err`: the cast's error message).
    files: Option<Result<NestedSet<Artifact>, String>>,
    stateless_runfiles: Option<Runfiles>,
    data_runfiles: Option<Runfiles>,
    default_runfiles: Option<Runfiles>,
    executable: Option<Artifact>,
    /// `info.getCreationLocation()`; `None` for `DefaultInfo.createEmpty(implLoc)`.
    creation_location: Option<String>,
}

impl DefaultInfoFields {
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java DefaultInfo.createEmpty
    fn empty() -> DefaultInfoFields {
        DefaultInfoFields {
            files: None,
            stateless_runfiles: None,
            data_runfiles: None,
            default_runfiles: None,
            executable: None,
            creation_location: None,
        }
    }

    fn of(default_info: &FrozenDefaultInfo) -> DefaultInfoFields {
        DefaultInfoFields {
            // Java: Depset.cast(files, Artifact.class, "files") in addSimpleProviders; the
            // conversion's error is reported there.
            files: default_info
                .get_files()
                .map(|v| depset_cast_artifacts(v, "files").map_err(|e| e.to_string())),
            stateless_runfiles: default_info.get_stateless_runfiles(),
            data_runfiles: default_info.get_data_runfiles(),
            default_runfiles: default_info.get_default_runfiles(),
            executable: default_info.get_executable(),
            creation_location: Some(default_info.get_creation_location().to_string()),
        }
    }
}

/// Returns an EvalException message with the info's creation location as a prefix. The message
/// is intended to be reported as ruleErrors by createTarget.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:331 StarlarkRuleConfiguredTargetUtil.infoError
fn info_error(creation_location: Option<&str>, rule_class: &RuleClass, msg: String) -> String {
    match creation_location {
        Some(loc) => format!("{loc}: {msg}"),
        // DefaultInfo.createEmpty(implLoc)
        None => format!("{}: {msg}", impl_location(rule_class)),
    }
}

/// `Info.getCreationLocation()` of a Starlark provider instance.
fn starlark_info_creation_location(info: Value) -> Option<String> {
    if let Some(d) = DefaultInfo::from_value(info) {
        return Some(d.get_creation_location().to_string());
    }
    // PORT: blaze_starlark's StarlarkInfo does not record its creation location.
    blaze_util::unported!("StarlarkInfo.getCreationLocation")
}

/// Returns the provider key from an info (provider instance), or the `infoError` message if the
/// provider for this info object has not been exported, which can occur if the provider was
/// declared in a non-global scope (for example a rule implementation function).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:342 StarlarkRuleConfiguredTargetUtil.getProviderKey
fn get_provider_key(info: Value) -> Result<ProviderKey, String> {
    match info_provider_key(info) {
        Some(key) => Ok(key),
        None => {
            // TODO(adonovan): report separate error events at distinct locations:
            //  "cannot return non-exported provider" (at location of instantiation), and
            //  "provider definition not at top level" (at location of definition).
            let loc = starlark_info_creation_location(info).unwrap_or_default();
            // Java: "(Provider defined at %s.)" with provider.getLocation()
            let _ = loc;
            blaze_util::unported!(
                "StarlarkRuleConfiguredTargetUtil.getProviderKey",
                "the rule implementation function returned an instance of an unnamed provider"
            )
        }
    }
}

/// A frozen element of the frozen raw providers, owned by the same heap.
fn owned_element(owner: &OwnedFrozenValue, v: Value) -> OwnedFrozenValue {
    let frozen: FrozenValue = v.unpack_frozen().expect("frozen provider");
    owner.map(|_| frozen)
}

/// `addProviders`: `Ok(Err(msg))` is Java's `EvalException` (reported by `createTarget`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:238 StarlarkRuleConfiguredTargetUtil.addProviders
fn add_providers(
    context: &RuleContext,
    rule_class: &RuleClass,
    builder: &mut RuleConfiguredTargetBuilder<'_>,
    raw_providers: &OwnedFrozenValue,
    is_default_executable_created: bool,
) -> Result<Result<(), String>, CreateConfiguredTargetError> {
    let raw = raw_providers.value();

    // Handle "return provider" vs "return [provider]"
    let mut declared_providers: IndexMap<ProviderKey, OwnedFrozenValue> = IndexMap::new();
    if is_info(raw) {
        let key = match get_provider_key(raw) {
            Ok(k) => k,
            Err(e) => return Ok(Err(e)),
        };
        if key == STRUCT.key() {
            let loc = starlark_info_creation_location(raw);
            return Ok(Err(info_error(
                loc.as_deref(),
                rule_class,
                "Returning a struct from a rule implementation function is deprecated.".to_owned(),
            )));
        }
        // A single declared provider (not in a list)
        // PORT: Java's StarlarkInfo.unsafeOptimizeMemoryLayout call here only changes the
        // memory layout of the provider instance (no observable effect); nothing to port.
        // Single declared provider
        declared_providers.insert(key, raw_providers.clone());
    } else if ListRef::from_value(raw).is_some()
        || TupleRef::from_value(raw).is_some()
        || raw.get_type() == "range"
    {
        // Sequence of declared providers
        let elements: Vec<Value> = match ListRef::from_value(raw) {
            Some(l) => l.content().to_vec(),
            None => match TupleRef::from_value(raw) {
                Some(t) => t.content().to_vec(),
                None => {
                    if raw.length().unwrap_or(0) > 0 {
                        return Ok(Err(
                            "at index 0 of result of rule implementation function, got element \
                             of type int, want Info"
                                .to_owned(),
                        ));
                    }
                    Vec::new()
                }
            },
        };
        // Java: Sequence.cast(rawProviders, Info.class, "result of rule implementation function")
        for (i, elem) in elements.iter().enumerate() {
            if !is_info(*elem) {
                return Ok(Err(format!(
                    "at index {i} of result of rule implementation function, got element of type \
                     {}, want Info",
                    elem.get_type()
                )));
            }
        }
        for provider in elements {
            // Provider instances are optimised recursively, without optimising elements of the
            // list. Tradeoff is that some object may be duplicated if they are reachable by more
            // than one path, but we don't expect that much in practice.
            let provider_key = match get_provider_key(provider) {
                Ok(k) => k,
                Err(e) => return Ok(Err(e)),
            };
            let owned = owned_element(raw_providers, provider);
            if declared_providers
                .insert(provider_key.clone(), owned)
                .is_some()
            {
                context.rule_error(&format!(
                    "Multiple conflicting returned providers with key {provider_key}"
                ));
            }
        }
    } else if !raw.is_none() {
        return Ok(Err(format!(
            "Expected a list of providers, but got {}",
            raw.get_type()
        )));
    }

    if rule_class.is_materializer_rule() {
        if declared_providers.len() != 1
            || !declared_providers.contains_key(&MATERIALIZED_DEPS_INFO.key())
        {
            let keys: Vec<String> = declared_providers.keys().map(|k| k.to_string()).collect();
            return Ok(Err(format!(
                "Materializer rules must return exactly one MaterializedDepsInfo provider, but \
                 got [{}]",
                keys.join(", ")
            )));
        }
    }

    let run_environment_info = declared_providers.get(&RUN_ENVIRONMENT_INFO.key()).cloned();
    if let Some(info) = &run_environment_info {
        if !rule_class.is_executable_starlark() && !context.is_test_target() {
            let message =
                "Returning RunEnvironmentInfo from a non-executable, non-test target has no effect";
            let should_error = downcast_info::<RunEnvironmentInfo>(info)
                .expect("RunEnvironmentInfo")
                .should_error_on_non_executable_rule();
            if should_error {
                context.rule_error(message);
                declared_providers.shift_remove(&RUN_ENVIRONMENT_INFO.key());
            } else {
                context.rule_warning(message);
            }
        }
    }

    let mut default_provider_provided_explicitly = false;
    for declared_provider in declared_providers.into_values() {
        if let Some(default_info) = downcast_info::<FrozenDefaultInfo>(&declared_provider) {
            let fields = DefaultInfoFields::of(default_info);
            match parse_default_provider_fields(
                fields,
                context,
                rule_class,
                builder,
                is_default_executable_created,
                run_environment_info.as_ref(),
            )? {
                Ok(()) => {}
                Err(e) => return Ok(Err(e)),
            }
            default_provider_provided_explicitly = true;
        } else {
            builder.add_starlark_declared_provider(declared_provider);
        }
    }

    if !default_provider_provided_explicitly {
        // TODO(b/308767456): Avoid creating an empty DefaultInfo, just to pass location for
        // throwing exceptions.
        if let Err(e) = parse_default_provider_fields(
            DefaultInfoFields::empty(),
            context,
            rule_class,
            builder,
            is_default_executable_created,
            run_environment_info.as_ref(),
        )? {
            return Ok(Err(e));
        }
    }
    Ok(Ok(()))
}

/// Parses fields of a default provider.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:359 StarlarkRuleConfiguredTargetUtil.parseDefaultProviderFields
fn parse_default_provider_fields(
    default_info: DefaultInfoFields,
    context: &RuleContext,
    rule_class: &RuleClass,
    builder: &mut RuleConfiguredTargetBuilder<'_>,
    is_default_executable_created: bool,
    run_environment_info: Option<&OwnedFrozenValue>,
) -> Result<Result<(), String>, CreateConfiguredTargetError> {
    let loc = default_info.creation_location.clone();
    let files = default_info.files;
    let mut stateless_runfiles = default_info.stateless_runfiles;
    let data_runfiles = default_info.data_runfiles;
    let default_runfiles = default_info.default_runfiles;
    let mut executable = default_info.executable;
    let rule_class_name = context.get_rule().get_rule_class().to_owned();

    if let Some(executable) = &executable {
        if executable.get_artifact_owner()
            != blaze_actions::ArtifactOwner::ActionLookupKey(context.get_owner().clone())
        {
            return Ok(Err(info_error(
                loc.as_deref(),
                rule_class,
                format!(
                    "'executable' provided by an executable rule '{rule_class_name}' should be \
                     created by the same rule."
                ),
            )));
        }
    }

    let is_executable = context
        .get_rule()
        .get_rule_class_object()
        .is_executable_starlark();
    if let Some(e) = &executable {
        if is_executable && is_default_executable_created {
            let default_executable = context.create_output_artifact();
            if e != &default_executable {
                return Ok(Err(info_error(
                    loc.as_deref(),
                    rule_class,
                    format!(
                        "The rule '{rule_class_name}' both accesses 'ctx.outputs.executable' and \
                         provides a different executable '{}'. Do not use \
                         'ctx.output.executable'.",
                        e.get_root_relative_path_string()
                    ),
                )));
            }
        }
    }

    if context.get_rule().is_analysis_test() {
        // The Starlark Build API should already throw exception if the rule implementation
        // attempts to register any actions. This is just a check of this invariant.
        blaze_util::check_state!(
            context
                .get_analysis_environment()
                .get_registered_actions()
                .is_empty(),
            "{}",
            context.get_label()
        );

        executable = Some(context.create_output_artifact_script_for_analysis_test());
    }

    if executable.is_none() && is_executable {
        if is_default_executable_created {
            // This doesn't actually create a new Artifact just returns the one
            // created in StarlarkRuleContext.
            executable = Some(context.create_output_artifact());
        } else {
            return Ok(Err(info_error(
                loc.as_deref(),
                rule_class,
                format!(
                    "The rule '{rule_class_name}' is executable. It needs to create an executable \
                     File and pass it as the 'executable' parameter to the DefaultInfo it \
                     returns."
                ),
            )));
        }
    }

    if stateless_runfiles.is_none() && data_runfiles.is_none() && default_runfiles.is_none() {
        // No runfiles specified, set default
        stateless_runfiles = Some(Runfiles::empty());
    }

    add_simple_providers(
        builder,
        context,
        executable,
        files,
        stateless_runfiles,
        data_runfiles,
        default_runfiles,
        run_environment_info,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:430 StarlarkRuleConfiguredTargetUtil.addSimpleProviders
fn add_simple_providers(
    builder: &mut RuleConfiguredTargetBuilder<'_>,
    rule_context: &RuleContext,
    executable: Option<Artifact>,
    files: Option<Result<NestedSet<Artifact>, String>>,
    stateless_runfiles: Option<Runfiles>,
    data_runfiles: Option<Runfiles>,
    default_runfiles: Option<Runfiles>,
    run_environment_info: Option<&OwnedFrozenValue>,
) -> Result<Result<(), String>, CreateConfiguredTargetError> {
    // TODO(bazel-team) if both 'files' and 'executable' are provided, 'files' overrides
    // 'executable'
    let mut files_to_build: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
    files_to_build.add_all(rule_context.get_output_artifacts());
    if let Some(executable) = &executable {
        files_to_build.add(executable.clone());
    }
    builder.set_files_to_build(files_to_build.build());

    if let Some(files) = files {
        let files = match files {
            Ok(files) => files,
            Err(e) => return Ok(Err(e)),
        };
        // If we specify files_to_build we don't have the executable in it by default.
        builder.set_files_to_build(files);
    }

    // This works because we only allowed to call a rule *_test iff it's a test type rule.
    let test_rule = is_test_rule_name(rule_context.get_rule().get_rule_class());
    let is_executable_or_test = executable.is_some() || test_rule;
    let runfiles_provider = match stateless_runfiles {
        Some(stateless_runfiles) => RunfilesProvider::simple(merge_files(
            stateless_runfiles,
            executable.as_ref(),
            rule_context,
        )),
        None => {
            let mut merged_default_runfiles = default_runfiles.unwrap_or_else(Runfiles::empty);
            if is_executable_or_test {
                // The executable is only merged in if needed when using stateful runfiles to
                // preserve long-standing behavior.
                merged_default_runfiles =
                    merge_files(merged_default_runfiles, executable.as_ref(), rule_context);
            }
            RunfilesProvider::with_data(
                merged_default_runfiles,
                data_runfiles.unwrap_or_else(Runfiles::empty),
            )
        }
    };
    let computed_default_runfiles = runfiles_provider.get_default_runfiles().clone();
    builder.add_provider(runfiles_provider);

    if test_rule && computed_default_runfiles.is_empty() {
        return Ok(Err("Test rules have to define runfiles".to_owned()));
    }
    if is_executable_or_test {
        let mut runfiles_support = None;
        if !computed_default_runfiles.is_empty() {
            let executable = executable
                .clone()
                .expect("NullPointerException: executable must not be null");
            runfiles_support = Some(Arc::new(crate::rule::runfiles_support::with_executable(
                rule_context,
                computed_default_runfiles,
                executable,
                run_environment_info,
            )?));
        }
        builder.set_runfiles_support(runfiles_support, executable);
    }

    if rule_context
        .get_rule()
        .get_rule_class_object()
        .is_starlark_testable()
    {
        // Java: ActionsProvider.create(ruleContext.getAnalysisEnvironment().getRegisteredActions())
        blaze_util::unported!("ActionsProvider.create");
    }
    Ok(Ok(()))
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:502 StarlarkRuleConfiguredTargetUtil.mergeFiles
fn merge_files(
    runfiles: Runfiles,
    executable: Option<&Artifact>,
    rule_context: &RuleContext,
) -> Runfiles {
    let Some(executable) = executable else {
        return runfiles;
    };
    let mut builder = RunfilesBuilder::new(rule_context.get_workspace_name());
    builder.add_artifact(executable.clone());
    builder.merge(&runfiles);
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ports of StarlarkRuleContextTest / StarlarkIntegrationTest cases that need the whole
    // analysis pipeline (rule context, configured targets, toolchains).
    macro_rules! needs_pipeline {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                #[ignore = "needs the analysis pipeline (RuleContext, ConfiguredTargetFunction)"]
                fn $name() {}
            )*
        };
    }

    needs_pipeline!(
        test_struct_returned_from_rule_is_deprecated,
        test_rule_returns_list_of_providers,
        test_multiple_conflicting_returned_providers_with_same_key,
        test_expected_list_of_providers,
        test_default_info_files_override_executable,
        test_executable_rule_without_executable,
        test_executable_from_different_rule,
        test_both_accesses_outputs_executable_and_provides_different_executable,
        test_test_rules_have_to_define_runfiles,
        test_run_environment_info_on_non_executable_rule,
        test_advertised_provider_not_returned,
        test_orphan_artifacts_reported,
        test_tree_artifacts_conflicting_with_files_reported,
        test_rule_should_return_struct_or_list,
        test_super_not_called,
    );

    #[test]
    fn artifacts_description_of_nothing_is_empty() {
        assert_eq!(artifacts_description(Vec::<&Artifact>::new()), "");
    }
}
