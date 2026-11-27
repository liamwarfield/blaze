//! SINGLE_EXTENSION: validates the result of SINGLE_EXTENSION_EVAL against the usages.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionFunction.java. The checks
//! after the lookups are `blaze_bzlmod::skyframe::check_single_extension`.

use blaze_bzlmod::skyframe::{SingleExtensionKey, SingleExtensionValue, check_single_extension};
use blaze_skyframe::{ComputeError, Environment};

use crate::bzlmod::single_extension_usages::SingleExtensionUsagesKey;

// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionFunction.java:37 SingleExtensionFunction.compute
pub fn compute(
    key: &SingleExtensionKey,
    env: &mut Environment<'_>,
) -> Result<Option<SingleExtensionValue>, ComputeError> {
    let extension_id = key.argument();
    let Some(usages_value) = env.get_value(SingleExtensionUsagesKey::key(extension_id.clone()))?
    else {
        return Ok(None);
    };
    let Some(eval_only_value) =
        env.get_value(SingleExtensionValue::eval_key(extension_id.clone()))?
    else {
        return Ok(None);
    };
    check_single_extension(
        extension_id,
        &eval_only_value,
        &usages_value.extension_usages,
        &usages_value.repo_overrides,
        env,
    )?;
    Ok(Some((*eval_only_value).clone()))
}
