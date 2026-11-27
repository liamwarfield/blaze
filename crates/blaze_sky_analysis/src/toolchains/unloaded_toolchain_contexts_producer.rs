//! Port of `analysis/producers/UnloadedToolchainContextsProducer.java` as a plain function over
//! the blocking evaluator (crates/blaze_analysis/DESIGN.md: producers become functions that
//! request dependencies in Java's order).

use std::sync::Arc;

use blaze_analysis::toolchains::{
    ToolchainCollection, ToolchainContextKey, ToolchainException, UnloadedToolchainContext,
    no_matching_platform_exception,
};
use blaze_packages::rule_class::DEFAULT_EXEC_GROUP_NAME;
use blaze_skyframe::{Environment, Interrupted, SkyKey};

use blaze_analysis::toolchains::UnloadedToolchainContextsInputs;

/// What the producer's `ResultSink` receives: `Ok(None)` when no toolchain resolution is
/// required (`acceptUnloadedToolchainContexts(null)`), `Ok(Some(..))` the contexts, `Err` the
/// first error (`acceptUnloadedToolchainContextsError`).
pub type UnloadedToolchainContextsResult =
    Result<Option<Arc<ToolchainCollection<UnloadedToolchainContext>>>, ToolchainException>;

/// Java `AspectFunction.BaseTargetPrerequisitesSupplier.getUnloadedToolchainContext` (aspect
/// evaluation only).
// PORT: a function instead of agent C's supplier type, to keep the crates decoupled.
pub type UnloadedToolchainContextSupplier<'a> =
    &'a dyn Fn(&ToolchainContextKey) -> Option<Arc<UnloadedToolchainContext>>;

/// Determines `ToolchainCollection<UnloadedToolchainContext>` from
/// [`UnloadedToolchainContextsInputs`]. Returns `Ok(None)` when a dependency is missing (Java:
/// the state machine does not complete).
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:63 UnloadedToolchainContextsProducer.<init>
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:37 UnloadedToolchainContextsProducer.ResultSink.acceptUnloadedToolchainContexts
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:40 UnloadedToolchainContextsProducer.ResultSink.acceptUnloadedToolchainContextsError
// PORT: the result sink is the return value.
pub fn produce_unloaded_toolchain_contexts(
    env: &mut Environment<'_>,
    unloaded_toolchain_contexts_inputs: &UnloadedToolchainContextsInputs,
) -> Result<Option<UnloadedToolchainContextsResult>, Interrupted> {
    produce_unloaded_toolchain_contexts_with_supplier(env, unloaded_toolchain_contexts_inputs, None)
}

/// [`produce_unloaded_toolchain_contexts`] with the aspect function's
/// `baseTargetPrerequisitesSupplier`.
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:89 UnloadedToolchainContextsProducer.step
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:77 UnloadedToolchainContextsProducer.<init>
pub fn produce_unloaded_toolchain_contexts_with_supplier(
    env: &mut Environment<'_>,
    unloaded_toolchain_contexts_inputs: &UnloadedToolchainContextsInputs,
    base_target_prerequisites_supplier: Option<UnloadedToolchainContextSupplier<'_>>,
) -> Result<Option<UnloadedToolchainContextsResult>, Interrupted> {
    let Some(default_toolchain_context_key) =
        unloaded_toolchain_contexts_inputs.target_toolchain_context_key()
    else {
        // Doesn't use toolchain resolution and short-circuits.
        return Ok(Some(Ok(None)));
    };

    // The lookups of the step, in order: the default exec group, then each exec group.
    let mut lookups: Vec<(String, ToolchainContextKey)> = Vec::new();
    lookups.push((
        DEFAULT_EXEC_GROUP_NAME.to_owned(),
        default_toolchain_context_key.clone(),
    ));
    let mut key_builder = ToolchainContextKey::key();
    key_builder
        .configuration_key(default_toolchain_context_key.configuration_key().clone())
        .debug_target(default_toolchain_context_key.debug_target());
    for (name, exec_group) in unloaded_toolchain_contexts_inputs.exec_groups() {
        let key = key_builder
            .toolchain_types(exec_group.toolchain_types())
            .exec_constraint_labels(exec_group.exec_compatible_with.iter().cloned())
            .build();
        lookups.push((name.clone(), key));
    }

    let mut toolchain_contexts_builder =
        ToolchainCollection::<UnloadedToolchainContext>::builder_with_expected_size(
            unloaded_toolchain_contexts_inputs.exec_groups().len() + 1,
        );
    let mut toolchain_contexts_error: Option<ToolchainException> = None;

    // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:129 UnloadedToolchainContextsProducer.lookupToolchainContext
    // Java: lookupToolchainContext. Contexts the supplier provides are accepted directly; the
    // others are looked up together (Tasks.lookUp).
    let mut supplied: Vec<Option<Arc<UnloadedToolchainContext>>> = Vec::new();
    let mut sky_keys: Vec<SkyKey> = Vec::new();
    for (_, key) in &lookups {
        let toolchain_context =
            base_target_prerequisites_supplier.and_then(|supplier| supplier(key));
        if toolchain_context.is_none() {
            sky_keys.push(SkyKey::new(key.clone()));
        }
        supplied.push(toolchain_context);
    }
    let results = env.get_values_and_exceptions(&sky_keys)?;

    for ((exec_group_name, key), supplied) in lookups.iter().zip(supplied) {
        // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:153 UnloadedToolchainContextsProducer.ToolchainContextLookupCallback.<init>
        // Java: ToolchainContextLookupCallback.acceptValueOrException.
        let value_or_error: Result<Option<Arc<UnloadedToolchainContext>>, ToolchainException> =
            match supplied {
                Some(v) => Ok(Some(v)),
                None => results.get_or_throw_typed::<ToolchainContextKey, ToolchainException>(key),
            };
        match value_or_error {
            Ok(Some(unloaded_toolchain_context)) => {
                if let Some(error_data) = unloaded_toolchain_context.error_data() {
                    handle_error(
                        &mut toolchain_contexts_error,
                        no_matching_platform_exception(error_data),
                    );
                    continue;
                }
                toolchain_contexts_builder
                    .add_context(exec_group_name, (*unloaded_toolchain_context).clone());
            }
            Err(error) => handle_error(&mut toolchain_contexts_error, error),
            // The dependency failed with another error: the state machine does not complete.
            Ok(None) => {}
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:185 UnloadedToolchainContextsProducer.buildToolchainContexts
    if let Some(error) = toolchain_contexts_error {
        return Ok(Some(Err(error)));
    }
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(Ok(Some(Arc::new(toolchain_contexts_builder.build())))))
}

// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsProducer.java:178 UnloadedToolchainContextsProducer.handleError
fn handle_error(first_error: &mut Option<ToolchainException>, error: ToolchainException) {
    if first_error.is_none() {
        // Only propagates the first error.
        *first_error = Some(error);
    }
}
