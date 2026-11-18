//! Port of `skyframe/toolchains/ConfigMatchingUtil.java`: helper class for
//! `ConfigMatchingProvider` data.

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::Label;

/// Validates that the given [`ConfigMatchingProvider`] instances are all successful.
///
/// `label` is the source of the instances, for error reporting; `error_handler`, if present,
/// receives an error message for each non-successful instance; `settings_attribute` is the name
/// of the attribute defining the config settings, for error reporting. Returns whether all
/// instances are successful, or an [`InvalidConfigurationException`] if any instances are in an
/// error state.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ConfigMatchingUtil.java:42 ConfigMatchingUtil.validate
pub fn validate(
    label: &Label,
    config_settings: &[ConfigMatchingProvider],
    error_handler: Option<&mut dyn FnMut(String)>,
    settings_attribute: &str,
) -> Result<bool, InvalidConfigurationException> {
    // Make sure the target setting matches but watch out for resolution errors.
    let accumulate_results = ConfigMatchingProvider::accumulate_match_results(config_settings);
    if !accumulate_results.errors.is_empty() {
        // TODO(blaze-configurability-team): This should only be due to feature flag trimming. So,
        // would be better to just ensure toolchain resolution isn't transitive.
        let message = accumulate_results
            .errors
            .iter()
            .map(|(key, values)| {
                format!(
                    "For config_setting {}: {}",
                    key.get_name(),
                    values.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(InvalidConfigurationException::new(format!(
            "Unrecoverable errors resolving config_setting associated with {label}: {message}"
        )));
    }
    if accumulate_results.success() {
        return Ok(true);
    } else if !accumulate_results.non_matching.is_empty()
        && let Some(error_handler) = error_handler
    {
        let mut distinct: Vec<&Label> = Vec::new();
        for l in &accumulate_results.non_matching {
            if !distinct.contains(&l) {
                distinct.push(l);
            }
        }
        let non_matching_list = distinct
            .iter()
            .map(|l| l.get_name())
            .collect::<Vec<_>>()
            .join(", ");
        let message = format!("mismatching {settings_attribute}: {non_matching_list}");
        error_handler(message);
    }
    Ok(false)
}
