//! Port of `skyframe/TargetPatternUtil.java`: helps with evaluating target patterns.
//!
//! PORT: Java keeps it in `lib/skyframe` next to the loading functions; its only analysis users
//! are the registered toolchains/execution platforms functions, so it lives here. Move to
//! `blaze_sky_loading` if loading code needs it.

use blaze_cmdline::{
    Label, SignedTargetPattern, SignedTargetPatternSign, TargetParsingException, TargetPattern,
    TargetPatternParser,
};
use blaze_sky_loading::target_pattern_function::{FilteringPolicy, TargetPatternKey};
use blaze_skyframe::{Environment, Interrupted, SkyKey};
use indexmap::IndexSet;

/// Expand the given `target_patterns`, using the `filtering_policy`. This handles the needed
/// underlying Skyframe calls (via `env`), and will return `Ok(Ok(None))` to signal a Skyframe
/// restart.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternUtil.java:44 TargetPatternUtil.expandTargetPatterns
pub fn expand_target_patterns(
    env: &mut Environment<'_>,
    target_patterns: &[SignedTargetPattern],
    filtering_policy: FilteringPolicy,
) -> Result<Result<Option<IndexSet<Label>>, InvalidTargetPatternException>, Interrupted> {
    if target_patterns.is_empty() {
        return Ok(Ok(Some(IndexSet::new())));
    }

    // Java: TargetPatternValue.keys(targetPatterns, filteringPolicy).
    let target_pattern_keys: Vec<TargetPatternKey> = target_patterns
        .iter()
        .map(|p| TargetPatternKey::key(p.clone(), filtering_policy.clone()))
        .collect();
    let sky_keys: Vec<SkyKey> = target_pattern_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let resolved_patterns = env.get_values_and_exceptions(&sky_keys)?;
    let values_missing = env.values_missing();

    // Use an ArrayList so that we can add and remove results based on negative patterns.
    let mut labels: Vec<Label> = Vec::new();

    for pattern in &target_pattern_keys {
        match resolved_patterns
            .get_or_throw_typed::<TargetPatternKey, TargetParsingException>(pattern)
        {
            Err(e) => {
                return Ok(Err(InvalidTargetPatternException::new(
                    pattern.get_pattern(),
                    e,
                )));
            }
            Ok(value) => {
                let Some(value) = value.filter(|_| !values_missing) else {
                    continue;
                };
                if pattern.is_negative() {
                    // Remove from the results.
                    // Java: ArrayList.removeAll.
                    labels.retain(|l| !value.get_targets().contains(l));
                } else {
                    // Add to results.
                    labels.extend(value.get_targets().iter().cloned());
                }
            }
        }
    }

    if env.values_missing() {
        // PORT: skipped, Google-internal: `if (valuesMissing != env.valuesMissing())
        // BugReport.logUnexpected("Some value from '%s' was missing, this should never happen",
        // targetPatternKeys)` only writes a log line; Java returns null here in both cases.
        return Ok(Ok(None));
    }

    // Java: ImmutableSet.copyOf(labels).
    Ok(Ok(Some(labels.into_iter().collect())))
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternUtil.java:90 TargetPatternUtil.parseAllSigned
pub fn parse_all_signed(
    patterns: &[String],
    parser: &TargetPatternParser,
) -> Result<Vec<SignedTargetPattern>, InvalidTargetPatternException> {
    let mut parsed_patterns = Vec::new();
    for pattern in patterns {
        match SignedTargetPattern::parse(pattern, parser) {
            Ok(p) => parsed_patterns.push(p),
            Err(e) => return Err(InvalidTargetPatternException::new(pattern, e)),
        }
    }
    Ok(parsed_patterns)
}

/// Converts patterns to signed patterns, considering all input patterns positive.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternUtil.java:104 TargetPatternUtil.toSigned
pub fn to_signed(patterns: Vec<TargetPattern>) -> Vec<SignedTargetPattern> {
    patterns
        .into_iter()
        .map(|pattern| SignedTargetPattern::create(pattern, SignedTargetPatternSign::Positive))
        .collect()
}

/// Exception used when an error occurs in [`expand_target_patterns`].
// Java: com/google/devtools/build/lib/skyframe/TargetPatternUtil.java:113 TargetPatternUtil.InvalidTargetPatternException
#[derive(Clone, Debug, thiserror::Error)]
#[error("com.google.devtools.build.lib.cmdline.TargetParsingException: {}", .tpe.get_message())]
pub struct InvalidTargetPatternException {
    invalid_pattern: String,
    tpe: TargetParsingException,
}

impl InvalidTargetPatternException {
    pub fn new(invalid_pattern: &str, tpe: TargetParsingException) -> Self {
        InvalidTargetPatternException {
            invalid_pattern: invalid_pattern.to_owned(),
            tpe,
        }
    }

    pub fn get_invalid_pattern(&self) -> &str {
        &self.invalid_pattern
    }

    pub fn get_tpe(&self) -> &TargetParsingException {
        &self.tpe
    }
}
