//! `select()` (Java `SelectorList.SelectLibrary.select` and the private static
//! `SelectorList.select` it calls).

use blaze_starlark::label::LabelValue;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;

use crate::build::eval_err;
use crate::package::TargetDefinitionContext;
use crate::types::LabelConverter;
use crate::types::SelectorList;
use crate::types::SelectorValue;

/// Builds the `select()` value: keys are kept as Labels, or label strings resolved with
/// `label_converter` when given.
// PORT: move to types/select.rs (A1) as `SelectorList::select`.
// Java: com/google/devtools/build/lib/packages/SelectorList.java:91 SelectorList.select
fn select_impl<'v>(
    dict: DictRef<'v>,
    no_match_error: &str,
    label_converter: Option<&LabelConverter>,
    heap: Heap<'v>,
) -> starlark::Result<SelectorList<'v>> {
    if dict.is_empty() {
        return Err(eval_err(
            "select({}) with an empty dictionary can never resolve because it includes no \
             conditions to match",
        ));
    }
    let mut select_dict: Vec<(Value<'v>, Value<'v>)> = Vec::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        let key = if key.downcast_ref::<LabelValue>().is_some() {
            key
        } else if let Some(label_string) = key.unpack_str() {
            match label_converter {
                Some(converter) => {
                    let label = converter.convert(label_string).map_err(|e| {
                        eval_err(format!("invalid label in select(): {}", e.get_message()))
                    })?;
                    heap.alloc(LabelValue(label))
                }
                None => key,
            }
        } else {
            return Err(eval_err(format!(
                "select: got {} for dict key, want a Label or label string",
                key.get_type()
            )));
        };
        // Java's ImmutableMap.Builder.buildOrThrow fails on duplicate keys (two strings that
        // resolve to the same Label).
        if select_dict
            .iter()
            .any(|(k, _)| k.equals(key).unwrap_or(false))
        {
            panic!("IllegalArgumentException: Multiple entries with same key: {key}");
        }
        select_dict.push((key, value));
    }
    // TODO(#26281): Tighten SelectorValue to accept an ImmutableMap<Label, Object> after
    //  flipping --incompatible_resolve_select_keys_eagerly.
    let selector = heap.alloc(SelectorValue::new(select_dict, no_match_error.to_owned()));
    Ok(SelectorList::of_selector(selector))
}

#[starlark_module]
pub(crate) fn select_library(builder: &mut GlobalsBuilder) {
    /// `select()` is the helper function that makes a rule attribute configurable.
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:255 SelectorList.SelectLibrary.select
    fn select<'v>(
        #[starlark(require = pos)] x: DictRef<'v>,
        #[starlark(require = named, default = "")] no_match_error: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // If this is not null, string keys in the dict will be resolved to Labels eagerly using
        // the given context.
        let heap = eval.heap();
        let resolve_eagerly = blaze_starlark::ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .incompatible_resolve_select_keys_eagerly;
        if !resolve_eagerly {
            return Ok(heap.alloc(select_impl(x, no_match_error, None, heap)?));
        }
        // The payload borrow is held only while converting keys and allocating (no Starlark
        // code runs).
        if let Some(ctx) = TargetDefinitionContext::from_or_null(eval) {
            // Handle the case of an initializer.
            // PORT: Java's `thread.getThreadLocal(LabelConverter.class)` is the builder's
            // initializer label converter while a rule initializer runs.
            let converter = match ctx.get_initializer_label_converter() {
                Some(c) => c.as_ref(),
                // Handle the case of a regular BUILD thread.
                None => ctx.get_label_converter(),
            };
            return Ok(heap.alloc(select_impl(x, no_match_error, Some(converter), heap)?));
        }
        // In all other cases, must be in a .bzl file.
        let label_converter = LabelConverter::for_bzl_evaluating_thread(eval).map_err(eval_err)?;
        Ok(heap.alloc(select_impl(
            x,
            no_match_error,
            Some(&label_converter),
            heap,
        )?))
    }
}
