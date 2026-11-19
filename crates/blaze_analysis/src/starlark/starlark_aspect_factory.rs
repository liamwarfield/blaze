//! Port of `skyframe/StarlarkAspectFactory.java`: a `ConfiguredAspectFactory` for aspects that
//! are defined in Starlark.
//!
//! The aspect implementation runs like a rule implementation
//! ([`super::starlark_rule_configured_target_util`]): in a fresh `Module`, with the analysis
//! thread payload; the module is frozen when the implementation returns (Java
//! `ruleContext.close()`), the actions it registered are built after freezing in registration
//! order, and the returned providers are frozen values that keep the module's heap alive.

use std::cell::RefCell;
use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryName};
use blaze_packages::aspect::AspectParameters;
use blaze_packages::starlark_defs::aspect::AspectData;
use blaze_packages::starlark_defs::builtin_providers::STRUCT;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::{EvalError, ThreadContext};
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::{FrozenValue, OwnedFrozenValue, ValueLike};

use super::starlark_action_factory::finish_pending_actions;
use super::starlark_rule_configured_target_util::analysis_thread_context;
use super::starlark_rule_context::StarlarkRuleContext;
use super::thread_payload::{
    AnalysisKeepAlive, AnalysisThreadPayload, FrozenAnalysisKeepAlive, RuleContextHandle,
};
use super::util::is_iterable;
use crate::configured_aspect::{ConfiguredAspect, ConfiguredAspectBuilder};
use crate::configured_aspect_factory::ConfiguredAspectFactory;
use crate::configured_rule_class_provider::CreateConfiguredTargetError;
use crate::configured_target::ConfiguredTarget;
use crate::configuredtargets::target_to_starlark;
use crate::info::{info_provider_key, is_info};
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::rule::RuleContext;
use crate::rule::config_util;
use crate::rule::configured_target_factory::{
    ConfiguredTargetFactory, ConfiguredTargetFactoryError,
};
use crate::rule::rule_error_consumer::RuleErrorConsumer;

/// A factory for aspects that are defined in Starlark.
// Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:39 StarlarkAspectFactory
pub struct StarlarkAspectFactory {
    /// Java `starlarkAspect` (the data of the `StarlarkDefinedAspect` value, which holds the
    /// implementation as a keep-alive handle).
    starlark_aspect: Arc<AspectData>,
}

/// What the evaluation of the implementation leaves behind once the module is frozen.
struct Evaluated {
    /// The frozen module (its extra value is the frozen keep-alive list).
    frozen: Option<FrozenModule>,
    /// Index of the implementation's result in the keep-alive list; `None` where Java returned
    /// `errorConfiguredAspect(ruleContext)` from the `try` (the error is on the rule context).
    aspect_starlark_object: Option<usize>,
    /// Java `requiredConfigFragments`, computed in the `finally` block before `close()`.
    required_config_fragments: Option<RequiredConfigFragmentsProvider>,
}

impl StarlarkAspectFactory {
    /// `starlark_aspect` is the `StarlarkDefinedAspect` (`aspect_data` of its value).
    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:42 StarlarkAspectFactory.<init>
    pub fn new(starlark_aspect: Arc<AspectData>) -> StarlarkAspectFactory {
        StarlarkAspectFactory { starlark_aspect }
    }

    /// Java `starlarkAspect.getImplementation()`, as an owned frozen value.
    fn get_implementation(&self) -> OwnedFrozenValue {
        self.starlark_aspect
            .get_implementation()
            .to_owned_frozen_value()
    }

    /// The `try`/`finally` of Java's `create`: initializes `ctx`, calls the implementation with
    /// `(ct, ctx)` in a fresh module, and freezes it.
    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:56 StarlarkAspectFactory.create
    fn evaluate(
        &self,
        thread: &ThreadContext,
        handle: RuleContextHandle,
        ct: &ConfiguredTarget,
    ) -> Evaluated {
        let opts = EvalOptions::default();
        let implementation = self.get_implementation();
        Module::with_temp_heap(|module| {
            let keep_alive = module.heap().alloc(AnalysisKeepAlive::new());
            module.set_extra_value(keep_alive);
            let keep = keep_alive
                .downcast_ref::<AnalysisKeepAlive>()
                .expect("keep-alive list");

            let aspect_starlark_object: Option<usize>;
            let required_config_fragments;
            // Java: a Depset created by the implementation wraps one NestedSet for as long as it
            // lives, before and after its Mutability is frozen; see rule::depsets.
            // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
            let depset_scope = crate::rule::depsets::DepsetConversionScope::enter();
            {
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(thread);
                eval.set_forbid_recursion(!thread.semantics.allow_recursion);
                // Java: net/starlark/java/eval/StringModule.java:326 and lib/json/Json.java:346 read
                // the thread's semantics (`--internal_starlark_utf_8_byte_strings`).
                eval.set_utf8_byte_strings(thread.semantics.utf8_byte_strings());
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");

                // Java: StarlarkRuleContext ctx = ruleContext.initStarlarkRuleContext();
                // Java: com/google/devtools/build/lib/analysis/RuleContext.java:996 RuleContext.initStarlarkRuleContext
                let aspect_descriptor = handle
                    .borrow()
                    .get_main_aspect()
                    .map(|a| a.get_descriptor().clone());
                match StarlarkRuleContext::create(handle, aspect_descriptor, module.heap()) {
                    Err(e) => {
                        // TODO(bazel-team): Doesn't this double-log the message, if the exception
                        // was created by RuleContext#throwWithRuleError?
                        handle
                            .borrow()
                            .rule_error(e.message.as_deref().unwrap_or("null"));
                        aspect_starlark_object = None;
                        required_config_fragments = handle.borrow().get_required_config_fragments();
                    }
                    Ok(ctx) => {
                        let ctx_value = module.heap().alloc(ctx);
                        let ctx_index = keep.push(ctx_value);
                        AnalysisThreadPayload::from_or_fail(&eval, "initStarlarkRuleContext")
                            .expect("analysis payload")
                            .starlark_rule_context = Some(ctx_index);
                        let ctx = ctx_value
                            .downcast_ref::<StarlarkRuleContext>()
                            .expect("ctx");

                        let semantics = handle.borrow().get_starlark_semantics().clone();
                        let target = target_to_starlark(ct, module.heap(), &semantics);
                        let function = eval.heap().access_owned_frozen_value(&implementation);
                        // Java: Starlark.positionalOnlyCall(thread, implementation, ct, ctx)
                        aspect_starlark_object =
                            match eval.eval_function(function, &[target, ctx_value], &[]) {
                                Ok(v) => Some(keep.push(v)),
                                Err(e) => {
                                    let ex = EvalError::from_starlark(&e);
                                    handle
                                        .borrow()
                                        .rule_error(&format!("\n{}", ex.message_with_stack()));
                                    None
                                }
                            };
                        // Java: finally { requiredConfigFragments =
                        //   ruleContext.getRequiredConfigFragments(); ruleContext.close(); }
                        required_config_fragments = handle.borrow().get_required_config_fragments();
                        ctx.close();
                    }
                }
            }
            // PORT: freezing copies the depsets the implementation made; the keep-alive list
            // gives the frozen copy of each one whose NestedSet an action already holds.
            // SAFETY: the depsets converted in the scope are values of `module`'s heap, which is
            // alive and is not garbage collected while a function call (the implementation)
            // runs.
            let converted_depsets: Vec<usize> = unsafe { depset_scope.converted_depsets() }
                .into_iter()
                .map(|depset| keep.push(depset))
                .collect();
            // Java: freeze mutability to allow optimizing StarlarkInfo instances
            let frozen = match module.freeze() {
                Ok(frozen) => {
                    // PORT: keep-alive (see starlark_rule_configured_target_util::evaluate).
                    crate::rule::depsets::keep_frozen_heap_alive(frozen.frozen_heap().clone());
                    let keep_alive = frozen
                        .owned_extra_value()
                        .expect("the keep-alive list is the module's extra value");
                    let frozen_depsets: Vec<OwnedFrozenValue> = converted_depsets
                        .iter()
                        .map(|index| FrozenAnalysisKeepAlive::get_owned(&keep_alive, *index))
                        .collect();
                    depset_scope.adopt_frozen(&frozen_depsets);
                    Some(frozen)
                }
                Err(e) => {
                    // PORT: Java's values are frozen by closing the Mutability, which cannot
                    // fail; starlark-rust refuses values that cannot be frozen.
                    handle.borrow().rule_error(&format!("\n{e:?}"));
                    None
                }
            };
            Evaluated {
                aspect_starlark_object: if frozen.is_some() {
                    aspect_starlark_object
                } else {
                    None
                },
                frozen,
                required_config_fragments,
            }
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:110 StarlarkAspectFactory.errorConfiguredAspect
    fn error_configured_aspect(
        rule_context: &RuleContext,
        required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredAspect>, CreateConfiguredTargetError> {
        match ConfiguredTargetFactory::errored_configured_aspect(
            rule_context,
            required_config_fragments_provider,
        ) {
            Ok(configured_aspect) => Ok(configured_aspect),
            Err(ConfiguredTargetFactoryError::Create(e)) => Err(e),
            Err(e) => panic!("erroredConfiguredAspect threw {e}"),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:117 StarlarkAspectFactory.createAspect
    fn create_aspect(
        aspect_starlark_object: &OwnedFrozenValue,
        rule_context: &RuleContext,
        required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Result<Option<ConfiguredAspect>, String>, CreateConfiguredTargetError> {
        let mut builder = ConfiguredAspectBuilder::new(rule_context);
        if let Some(required_config_fragments) = required_config_fragments {
            builder.add_provider(required_config_fragments);
        }
        let object = aspect_starlark_object.value();
        // not instanceof Info, because OutputGroupInfo is both Iterable and Info
        if !is_info(object) && is_iterable(object) {
            if let Err(e) = Self::add_declared_providers(&mut builder, aspect_starlark_object) {
                return Ok(Err(e));
            }
        } else {
            // A single declared provider (not in a list)
            // PORT: StarlarkInfo.unsafeOptimizeMemoryLayout only changes the memory layout.
            if let Err(e) = builder.add_starlark_declared_provider(aspect_starlark_object.clone()) {
                return Ok(Err(e.to_string()));
            }
        }

        let configured_aspect = builder.build()?;
        if let Err(e) = validate_artifacts(rule_context) {
            return Ok(Err(e));
        }
        Ok(Ok(configured_aspect))
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:145 StarlarkAspectFactory.addDeclaredProviders
    fn add_declared_providers(
        builder: &mut ConfiguredAspectBuilder,
        aspect_starlark_object: &OwnedFrozenValue,
    ) -> Result<(), String> {
        let elements: Vec<FrozenValue> = Module::with_temp_heap(|module| {
            module
                .heap()
                .access_owned_frozen_value(aspect_starlark_object)
                .iterate(module.heap())
                .map(|it| {
                    it.map(|v| {
                        v.unpack_frozen()
                            .expect("elements of a frozen value are frozen")
                    })
                    .collect()
                })
                .map_err(|e| e.to_string())
        })?;
        for (i, o) in elements.into_iter().enumerate() {
            let o_value = o.to_value();
            if !is_info(o_value) {
                return Err(format!(
                    "A return value of an aspect implementation function should be a sequence \
                     of declared providers, instead got a {} at index {}",
                    o_value.get_type(),
                    i
                ));
            }
            // PORT: StarlarkInfo.unsafeOptimizeMemoryLayout only changes the memory layout.
            if let Err(e) =
                builder.add_starlark_declared_provider(aspect_starlark_object.map(|_| o))
            {
                return Err(e.to_string());
            }
        }
        Ok(())
    }
}

impl ConfiguredAspectFactory for StarlarkAspectFactory {
    // Java: com/google/devtools/build/lib/skyframe/StarlarkAspectFactory.java:47 StarlarkAspectFactory.create
    fn create(
        &self,
        _target_label: &Label,
        ct: &ConfiguredTarget,
        rule_context: &mut RuleContext,
        _parameters: &AspectParameters,
        _tools_repository: &RepositoryName,
    ) -> Result<Option<ConfiguredAspect>, CreateConfiguredTargetError> {
        let thread = analysis_thread_context(rule_context);

        let cell: RefCell<&mut RuleContext> = RefCell::new(rule_context);
        // SAFETY: `cell` outlives the evaluation below, which is the only user of the handle:
        // every value holding it lives on the module's heap (dropped at the end of
        // `with_temp_heap`) or freezes without it, and the payload is taken back before `cell`
        // is dropped.
        let handle = unsafe { RuleContextHandle::new(&cell) };
        let thread = thread.with_payload(AnalysisThreadPayload::new(handle));

        let evaluated = self.evaluate(&thread, handle, ct);

        let payload = thread
            .take_payload::<AnalysisThreadPayload>()
            .expect("the analysis payload is only taken here");
        let rule_context: &mut RuleContext = cell.into_inner();

        // Java registered each action as it was created; build them now that their Starlark
        // values are frozen, in the same order.
        if let Some(frozen) = &evaluated.frozen {
            let keep_alive = frozen
                .owned_extra_value()
                .expect("the keep-alive list is the module's extra value");
            finish_pending_actions(payload.pending_actions, &keep_alive, rule_context);
        }

        let aspect_starlark_object = match (&evaluated.frozen, evaluated.aspect_starlark_object) {
            (Some(frozen), Some(index)) => {
                let keep_alive = frozen.owned_extra_value().expect("keep-alive list");
                Some(FrozenAnalysisKeepAlive::get_owned(&keep_alive, index))
            }
            _ => None,
        };
        let required_config_fragments = evaluated.required_config_fragments;
        let Some(aspect_starlark_object) = aspect_starlark_object else {
            // Java: return errorConfiguredAspect(ruleContext) from the catch blocks.
            let fragments = rule_context.get_required_config_fragments();
            return Self::error_configured_aspect(rule_context, fragments);
        };

        // If allowing analysis failures, targets should be created somewhat normally, and errors
        // will be propagated via a hook elsewhere as AnalysisFailureInfo.
        let allow_analysis_failures =
            config_util::allow_analysis_failures(rule_context.get_configuration());

        let object = aspect_starlark_object.value();
        if rule_context.has_errors() && !allow_analysis_failures {
            return Self::error_configured_aspect(rule_context, required_config_fragments);
        } else if is_info(object) && info_provider_key(object) == Some(STRUCT.key()) {
            rule_context.rule_error(
                "Returning a struct from an aspect implementation function is deprecated.",
            );
        } else if !is_iterable(object) && !is_info(object) {
            rule_context.rule_error(&format!(
                "Aspect implementation should return a list, or a provider instance, but got {}",
                object.get_type()
            ));
            return Self::error_configured_aspect(rule_context, required_config_fragments);
        }
        match Self::create_aspect(
            &aspect_starlark_object,
            rule_context,
            required_config_fragments.clone(),
        )? {
            Ok(configured_aspect) => Ok(configured_aspect),
            Err(e) => {
                rule_context.rule_error(&format!("\n{e}"));
                Self::error_configured_aspect(rule_context, required_config_fragments)
            }
        }
    }
}

/// `StarlarkProviderValidationUtil.validateArtifacts(ruleContext)` (an `EvalException` message
/// on failure).
// Java: com/google/devtools/build/lib/analysis/StarlarkProviderValidationUtil.java:25 StarlarkProviderValidationUtil.validateArtifacts
fn validate_artifacts(rule_context: &RuleContext) -> Result<(), String> {
    super::starlark_rule_configured_target_util::validate_artifacts(rule_context)
}
