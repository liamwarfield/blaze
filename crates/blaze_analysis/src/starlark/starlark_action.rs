//! The Starlark `Action` value (Java: every `AbstractAction` is a StarlarkValue implementing
//! `starlarkbuildapi/ActionApi.java`), as `target.actions` returns it.
//!
//! The value wraps the shared Rust action. The getters whose implementation varies by action
//! class (`argv`, `args`, `content`, `substitutions`) are the `blaze_actions::Action` trait
//! methods each action class overrides; the others are AbstractAction's final implementations.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::actions::action::ActionApiError;
use blaze_actions::{ActionRef, Artifact};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_starlark::ThreadContext;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::Hashed;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark_map::StarlarkHasher;

use super::util::{alloc_immutable_dict, alloc_immutable_list, errorf};
use crate::rule::depsets::{artifacts_to_depset, depset_value};

/// An action as Starlark sees it (type `Action`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/ActionApi.java:30 ActionApi
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:68 AbstractAction
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkAction {
    #[allocative(skip)]
    action: ActionRef,
}

starlark_simple_value!(StarlarkAction);

impl fmt::Debug for StarlarkAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StarlarkAction").field(&self.action).finish()
    }
}

impl fmt::Display for StarlarkAction {
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:426 AbstractAction.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.action.pretty_print())
    }
}

// The `@StarlarkMethod(structField = true)` names of ActionApi, in Java's `dir` order (sorted).
const ARGS: &str = "args";
const ARGV: &str = "argv";
const CONTENT: &str = "content";
const ENV: &str = "env";
// `enableOnlyWithFlag = BuildLanguageOptions.EXPERIMENTAL_GOOGLE_LEGACY_API`.
const EXECUTION_INFO: &str = "execution_info";
const INPUTS: &str = "inputs";
const MNEMONIC: &str = "mnemonic";
const OUTPUTS: &str = "outputs";
const SUBSTITUTIONS: &str = "substitutions";

fn api_error(e: ActionApiError) -> starlark::Error {
    match e {
        ActionApiError::Eval(message) => errorf(message),
        ActionApiError::Interrupted(_) => {
            blaze_util::unported!("InterruptedException thrown by an ActionApi getter")
        }
    }
}

impl StarlarkAction {
    pub fn new(action: ActionRef) -> StarlarkAction {
        StarlarkAction { action }
    }

    /// The wrapped action (Java: the `Action` object itself, e.g. for
    /// `((Action) a).getPrimaryOutput()`).
    pub fn action(&self) -> &ActionRef {
        &self.action
    }

    /// The action of a Starlark `Action` value.
    pub fn action_of<'v>(v: Value<'v>) -> Option<&'v ActionRef> {
        v.downcast_ref::<StarlarkAction>().map(|a| &a.action)
    }

    /// The value of the struct field `name`, `Ok(None)` for a name that is not a field.
    /// `experimental_google_legacy_api` enables `execution_info`.
    fn get_field<'v>(
        &self,
        name: &str,
        experimental_google_legacy_api: bool,
        heap: Heap<'v>,
    ) -> starlark::Result<Option<Value<'v>>> {
        let action = &self.action;
        Ok(Some(match name {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java getMnemonic
            MNEMONIC => heap.alloc_str(&action.get_mnemonic()).to_value(),
            // Java: com/google/devtools/build/lib/actions/AbstractAction.java:613 AbstractAction.getStarlarkInputs
            INPUTS => depset_value(&artifacts_to_depset(&action.get_inputs())),
            // Java: com/google/devtools/build/lib/actions/AbstractAction.java:618 AbstractAction.getStarlarkOutputs
            OUTPUTS => depset_value(&artifacts_to_depset(&NestedSetBuilder::<Artifact>::wrap(
                Order::STABLE_ORDER,
                action.get_outputs(),
            ))),
            // `allowReturnNones = true`: null is None.
            ARGV => match action.get_starlark_argv().map_err(api_error)? {
                Some(argv) => alloc_immutable_list(
                    heap,
                    argv.iter().map(|s| heap.alloc_str(s).to_value()).collect(),
                ),
                None => Value::new_none(),
            },
            ARGS => match action.get_starlark_args() {
                // Java: StarlarkList.immutableCopyOf(result).
                Some(args) => {
                    alloc_immutable_list(heap, args.into_iter().map(|a| heap.alloc(a)).collect())
                }
                None => Value::new_none(),
            },
            CONTENT => match action.get_starlark_content().map_err(api_error)? {
                Some(content) => heap.alloc_str(&content).to_value(),
                None => Value::new_none(),
            },
            SUBSTITUTIONS => match action.get_starlark_substitutions().map_err(api_error)? {
                // Java: Dict.immutableCopyOf(builder.buildOrThrow()).
                Some(substitutions) => alloc_immutable_dict(
                    heap,
                    substitutions
                        .iter()
                        .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
                        .collect(),
                ),
                None => Value::new_none(),
            },
            // Java: com/google/devtools/build/lib/actions/AbstractAction.java:654 AbstractAction.getEnv
            ENV => alloc_immutable_dict(
                heap,
                action
                    .get_environment()
                    .get_fixed_env()
                    .iter()
                    .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
                    .collect(),
            ),
            // Java: com/google/devtools/build/lib/actions/AbstractAction.java:648 AbstractAction.getExecutionInfoDict
            EXECUTION_INFO if experimental_google_legacy_api => alloc_immutable_dict(
                heap,
                action
                    .get_execution_info()
                    .iter()
                    .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
                    .collect(),
            ),
            _ => return Ok(None),
        }))
    }
}

/// Java `RuleConfiguredTarget`/`MergedConfiguredTarget.rawGetStarlarkProvider("actions")` as
/// `AbstractConfiguredTarget.getValue` returns it (`Starlark.fromJava(list, null)`: an immutable
/// list) of `actions`.
///
/// PORT: Java keeps only the actions that are `ActionApi` instances; every action class Blaze
/// ports as a `blaze_actions::Action` extends `AbstractAction` in Java (action templates are not
/// `Action`s here), so none is filtered out.
pub fn actions_to_starlark<'v>(actions: &[ActionRef], heap: Heap<'v>) -> Value<'v> {
    alloc_immutable_list(
        heap,
        actions
            .iter()
            .map(|a| heap.alloc(StarlarkAction::new(a.clone())))
            .collect(),
    )
}

#[starlark_value(type = "Action")]
impl<'v> StarlarkValue<'v> for StarlarkAction {
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:426 AbstractAction.repr
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.action.pretty_print());
    }

    /// Rust callers without an evaluator: the default semantics (`execution_info` disabled),
    /// `None` where Java throws.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_field(attribute, false, heap).ok().flatten()
    }

    fn get_attr_fallible(
        &self,
        attribute: Hashed<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        let experimental_google_legacy_api = ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .experimental_google_legacy_api;
        self.get_field(attribute.key(), experimental_google_legacy_api, eval.heap())
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr (a struct field method
    // enabled by the semantics).
    fn has_attr_fallible(
        &self,
        attribute: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let experimental_google_legacy_api = ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .experimental_google_legacy_api;
        Ok(match attribute {
            ARGS | ARGV | CONTENT | ENV | INPUTS | MNEMONIC | OUTPUTS | SUBSTITUTIONS => true,
            EXECUTION_INFO => experimental_google_legacy_api,
            _ => false,
        })
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            ARGS | ARGV | CONTENT | ENV | INPUTS | MNEMONIC | OUTPUTS | SUBSTITUTIONS
        )
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir (the annotated methods enabled
    // by the semantics). PORT: `dir` has no evaluator here, so the default semantics apply
    // (`execution_info`, enabled only with --experimental_google_legacy_api, is not listed).
    fn dir_attr(&self) -> Vec<String> {
        [
            ARGS,
            ARGV,
            CONTENT,
            ENV,
            INPUTS,
            MNEMONIC,
            OUTPUTS,
            SUBSTITUTIONS,
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }

    // Java: AbstractAction.isImmutable ("immutable and Starlark-hashable"); AbstractAction does
    // not override equals/hashCode, so identity.
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (Arc::as_ptr(&self.action) as *const () as usize).hash(hasher);
        Ok(())
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkAction>()
            .is_some_and(|o| std::ptr::addr_eq(Arc::as_ptr(&self.action), Arc::as_ptr(&o.action))))
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}
