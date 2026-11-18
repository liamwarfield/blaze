//! Port of `analysis/producers/PlatformProducer.java` as a plain function: retrieves the
//! `PlatformValue` for a given platform.

use std::sync::Arc;

use blaze_analysis::platform::{PlatformInfo, platform_provider_utils};
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_build_config::platform::PlatformValueKey;
use blaze_cmdline::Label;
use blaze_options::OptionsParsingException;
use blaze_skyframe::{Environment, Interrupted};

use crate::platform::platform_function::PlatformFunction;

/// The errors of the producer's `ResultSink`.
#[derive(Clone, Debug, thiserror::Error)]
pub enum PlatformProducerError {
    /// `acceptPlatformInfoError` (an `InvalidPlatformException`).
    #[error("{0}")]
    InvalidPlatform(ToolchainException),
    /// `acceptOptionsParsingError`: the `OptionsParsingException`'s message.
    #[error("{0}")]
    OptionsParsing(String),
}

enum Caught {
    Platform(ToolchainException),
    Options(OptionsParsingException),
}

/// Looks up `PlatformValue.key(platformLabel, flagAliasMappings)` and returns the platform's
/// `PlatformInfo` (Java `acceptPlatformValue(value)` then `value.platformInfo()`). `Ok(None)`
/// when the value is missing (another error).
///
/// PORT: the configuration seam's `PlatformValue` (`blaze_build_config::platform`) does not
/// carry the `PlatformInfo`; it is read from the platform's configured target, which
/// PLATFORM already requested (the same memoized `CONFIGURED_TARGET` node).
// Java: com/google/devtools/build/lib/analysis/producers/PlatformProducer.java:61 PlatformProducer.step
pub fn produce_platform(
    env: &mut Environment<'_>,
    platform_label: &Label,
    flag_alias_mappings: &[(String, Label)],
) -> Result<Option<Result<Arc<PlatformInfo>, PlatformProducerError>>, Interrupted> {
    let key = PlatformValueKey {
        platform_label: platform_label.clone(),
        flag_alias_mappings: flag_alias_mappings.to_vec(),
    };
    // Java: com/google/devtools/build/lib/analysis/producers/PlatformProducer.java:71 PlatformProducer.acceptValueOrException2
    let result = env.get_value_or_throw_with(key, |e| {
        if let Some(t) = e
            .downcast_ref::<ToolchainException>()
            .filter(|t| t.kind() == ToolchainExceptionKind::InvalidPlatform)
        {
            return Some(Caught::Platform(t.clone()));
        }
        e.downcast_ref::<OptionsParsingException>()
            .map(|o| Caught::Options(o.clone()))
    })?;
    match result {
        Ok(Some(_platform_value)) => {}
        Ok(None) => return Ok(None),
        Err(Caught::Platform(e)) => {
            return Ok(Some(Err(PlatformProducerError::InvalidPlatform(e))));
        }
        Err(Caught::Options(e)) => {
            return Ok(Some(Err(PlatformProducerError::OptionsParsing(
                e.to_string(),
            ))));
        }
    }

    // Java: PlatformValue.platformInfo().
    let Some(ctv) = env.get_value(PlatformFunction::configured_target_dep(platform_label))? else {
        return Ok(None);
    };
    let platform_info = platform_provider_utils::platform(Some(ctv.get_configured_target()))
        .expect("PLATFORM succeeded, so the platform provides PlatformInfo");
    Ok(Some(Ok(Arc::new(platform_info))))
}
