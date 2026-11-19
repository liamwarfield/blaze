//! Port of `analysis/producers/PlatformProducer.java` as a plain function: retrieves the
//! `PlatformValue` for a given platform.

use std::sync::Arc;

use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_build_config::platform::PlatformValueKey;
use blaze_cmdline::Label;
use blaze_options::OptionsParsingException;
use blaze_skyframe::{Environment, Interrupted};

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
/// PORT: the configuration seam's `PlatformValue` (`blaze_build_config::platform`) carries the
/// `PlatformInfo` as an opaque value (the type is defined in blaze_analysis); it is downcast
/// here. No other Skyframe request is made: the requesting node depends on PLATFORM only, as
/// in Java.
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
    let platform_value = match result {
        Ok(Some(platform_value)) => platform_value,
        Ok(None) => return Ok(None),
        Err(Caught::Platform(e)) => {
            return Ok(Some(Err(PlatformProducerError::InvalidPlatform(e))));
        }
        Err(Caught::Options(e)) => {
            return Ok(Some(Err(PlatformProducerError::OptionsParsing(
                e.to_string(),
            ))));
        }
    };

    // Java: PlatformValue.platformInfo().
    let platform_info = platform_value
        .platform_info
        .as_ref()
        .and_then(|info| info.0.clone().downcast::<PlatformInfo>().ok())
        .unwrap_or_else(|| {
            // A PlatformValue recorded from the oracle (BLAZE_W4_ORACLE_PLATFORMS) has no
            // PlatformInfo.
            blaze_util::unported!(
                "PlatformValue.platformInfo",
                "platform {} comes from recorded oracle platforms",
                platform_label
            )
        });
    Ok(Some(Ok(platform_info)))
}
