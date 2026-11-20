//! Port of `skyframe/LoadAspectsFunction.java`: the `LOAD_ASPECTS` SkyFunction, which loads
//! top level aspects and assigns their parameters. Also `skyframe/
//! TopLevelAspectsDetailsBuildFailedException.java`.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::{LOAD_ASPECTS, LoadAspectsKey, LoadAspectsValue};
use blaze_events::Event;
use blaze_packages::aspect::{Aspect, AspectClass, StarlarkAspectClass};
use blaze_packages::exceptions::DetailedExitCode;
use blaze_packages::rule_class::StarlarkCallable;
use blaze_packages::rule_class::aspects_list::AspectsListBuilder;
use blaze_sky_loading::bzl_load::{BzlLoadFailedException, BzlLoadKey};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, SkyFunction, SkyFunctionException, SkyKey,
    Transience,
};
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::aspect_function::{bzl_load_key_for_starlark_aspect, load_aspect_from_bzl};
use crate::producers::errors::create_aspect_detailed_exit_code;

/// Exception that encapsulates the ones that's thrown when evaluating the top level aspects.
// Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsDetailsBuildFailedException.java:23 TopLevelAspectsDetailsBuildFailedException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct TopLevelAspectsDetailsBuildFailedException {
    message: String,
    detailed_exit_code: DetailedExitCode,
}

impl TopLevelAspectsDetailsBuildFailedException {
    /// `code` is the name of an `Analysis.Code`, e.g. `ASPECT_CREATION_FAILED`.
    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsDetailsBuildFailedException.java:27 TopLevelAspectsDetailsBuildFailedException.<init>
    pub fn new(error_message: impl Into<String>, code: &str) -> Self {
        let message = error_message.into();
        TopLevelAspectsDetailsBuildFailedException {
            detailed_exit_code: create_aspect_detailed_exit_code(&message, code),
            message,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsDetailsBuildFailedException.java:38 TopLevelAspectsDetailsBuildFailedException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

/// SkyFunction to load top level aspects and assign their parameters.
// Java: com/google/devtools/build/lib/skyframe/LoadAspectsFunction.java:35 LoadAspectsFunction
pub struct LoadAspectsFunction;

/// Registers `LOAD_ASPECTS`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:874 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.function(LOAD_ASPECTS, LoadAspectsFunction)
}

fn throw(e: TopLevelAspectsDetailsBuildFailedException) -> ComputeError {
    // Java: LoadAspectsFunction.LoadAspectsFunctionException
    SkyFunctionException::new(e, Transience::Persistent).into()
}

impl SkyFunction for LoadAspectsFunction {
    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsFunction.java:39 LoadAspectsFunction.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> blaze_skyframe::ComputeResult {
        let top_level_aspects_details_key =
            key.downcast::<LoadAspectsKey>().expect("LOAD_ASPECTS key");

        let Some(top_level_aspects) = get_top_level_aspects(
            env,
            top_level_aspects_details_key.get_top_level_aspects_classes(),
            top_level_aspects_details_key.get_top_level_aspects_parameters(),
        )?
        else {
            return Ok(None); // some aspects are not loaded
        };

        Ok(Some(
            Arc::new(LoadAspectsValue::new(top_level_aspects)) as blaze_skyframe::SkyValue
        ))
    }
}

/// `Ok(None)` when the .bzl is not loaded yet.
// Java: com/google/devtools/build/lib/skyframe/LoadAspectsFunction.java:57 LoadAspectsFunction.loadStarlarkAspect
fn load_starlark_aspect(
    env: &mut Environment<'_>,
    aspect_class: &StarlarkAspectClass,
) -> Result<Option<OwnedFrozenValue>, ComputeError> {
    let message = match env.get_value_or_throw::<BzlLoadKey, BzlLoadFailedException>(
        bzl_load_key_for_starlark_aspect(aspect_class),
    )? {
        Ok(None) => return Ok(None),
        Ok(Some(bzl_load_value)) => match load_aspect_from_bzl(aspect_class, &bzl_load_value) {
            Ok(starlark_aspect) => return Ok(Some(starlark_aspect)),
            Err(e) => e.get_message().to_owned(),
        },
        Err(e) => e.get_message().to_owned(),
    };
    env.listener().handle(&Event::error(message.clone()));
    Err(throw(TopLevelAspectsDetailsBuildFailedException::new(
        message,
        "ASPECT_CREATION_FAILED",
    )))
}

// Java: com/google/devtools/build/lib/skyframe/LoadAspectsFunction.java:80 LoadAspectsFunction.getTopLevelAspects
fn get_top_level_aspects(
    env: &mut Environment<'_>,
    top_level_aspects_classes: &[AspectClass],
    top_level_aspects_parameters: &IndexMap<String, String>,
) -> Result<Option<Vec<Aspect>>, ComputeError> {
    let mut builder = AspectsListBuilder::new();

    for aspect_class in top_level_aspects_classes {
        match aspect_class {
            AspectClass::Starlark(starlark_aspect_class) => {
                let Some(starlark_aspect) = load_starlark_aspect(env, starlark_aspect_class)?
                else {
                    return Ok(None);
                };
                // PORT: keep-alive (the .bzl module's heap lives in the build arena).
                // SAFETY: BZL_LOAD keeps the module's frozen heap in the build arena, which
                // outlives this evaluator and its graph, the holders of the callable (see
                // `StarlarkCallable`).
                let callable =
                    StarlarkCallable::new(unsafe { starlark_aspect.unchecked_frozen_value() });
                if let Err(e) = builder.add_starlark_aspect(callable) {
                    // PORT: `Event.error(e.getInnermostLocation(), e.getMessageWithStack())`;
                    // the EvalException `AspectsList` raises carries neither a location nor a
                    // call stack.
                    env.listener().handle(&Event::error(e.get_message()));
                    return Err(throw(TopLevelAspectsDetailsBuildFailedException::new(
                        e.get_message(),
                        "ASPECT_CREATION_FAILED",
                    )));
                }
            }
            // PORT: Java catches the AssertionError `addAspect(NativeAspectClass)` can raise;
            // the Rust builder does not fail.
            AspectClass::Native(native_aspect_class) => {
                builder.add_native_aspect(native_aspect_class.clone());
            }
        }
    }

    let aspects_list = builder.build();
    let result = aspects_list
        .validate_top_level_aspects_parameters(top_level_aspects_parameters)
        .and_then(|()| aspects_list.build_aspects(top_level_aspects_parameters));
    match result {
        Ok(aspects) => Ok(Some(aspects)),
        Err(e) => {
            // PORT: see above for the event's location and stack.
            env.listener().handle(&Event::error(e.get_message()));
            Err(throw(TopLevelAspectsDetailsBuildFailedException::new(
                e.get_message(),
                "ASPECT_CREATION_FAILED",
            )))
        }
    }
}
