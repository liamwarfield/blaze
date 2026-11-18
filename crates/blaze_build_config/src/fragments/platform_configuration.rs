//! Port of `analysis/PlatformConfiguration.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_optdefs::fragment::regex_filter::RegexFilter;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::PLATFORM_OPTIONS;

// Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:33 PlatformConfiguration
/// A configuration fragment describing the current platform configuration.
#[derive(Debug)]
pub struct PlatformConfiguration {
    host_platform: Label,
    extra_execution_platforms: Vec<String>,
    target_platform: Label,
    extra_toolchains: Vec<String>,
    toolchain_resolution_debug_regex_filter: RegexFilter,
}

/// `PlatformConfiguration.class`.
pub static PLATFORM_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.PlatformConfiguration",
    starlark_name: Some("platform"),
    required_options: || vec![&PLATFORM_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(PlatformConfiguration::new(options));
    Ok(fragment)
}

// All characters with special meaning anywhere in a regex (':' for example is only special
// within brackets).
// Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:125 PlatformConfiguration.REGEX_SPECIAL_CHARS
const REGEX_SPECIAL_CHARS: &[char] = &['+', '.', '|', '(', '[', '{', '^', '$', '?', '\\', '*'];

impl PlatformConfiguration {
    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:40 PlatformConfiguration.<init>
    pub fn new(options: &BuildOptions) -> PlatformConfiguration {
        Self::from_platform_options(required::<PlatformOptions>(options))
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:40 PlatformConfiguration.<init>
    pub fn from_platform_options(platform_options: &PlatformOptions) -> PlatformConfiguration {
        PlatformConfiguration {
            host_platform: platform_options.host_platform.get().clone(),
            extra_execution_platforms: platform_options.extra_execution_platforms.clone(),
            target_platform: platform_options.compute_target_platform().clone(),
            extra_toolchains: platform_options.extra_toolchains.clone(),
            toolchain_resolution_debug_regex_filter: platform_options
                .toolchain_resolution_debug
                .clone(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:66 PlatformConfiguration.getHostPlatform
    pub fn get_host_platform(&self) -> &Label {
        &self.host_platform
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:74 PlatformConfiguration.getExtraExecutionPlatforms
    /// Target patterns that select additional platforms that will be made available for action
    /// execution.
    pub fn get_extra_execution_platforms(&self) -> &[String] {
        &self.extra_execution_platforms
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:84 PlatformConfiguration.getTargetPlatform
    /// Returns the single target platform used in this configuration. The flag is multi-valued for
    /// future handling of multiple target platforms but any given configuration should only be
    /// concerned with a single target platform.
    pub fn get_target_platform(&self) -> &Label {
        &self.target_platform
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:89 PlatformConfiguration.getTargetPlatforms
    pub fn get_target_platforms(&self) -> Vec<Label> {
        vec![self.target_platform.clone()]
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:97 PlatformConfiguration.getExtraToolchains
    /// Target patterns that select additional toolchains that will be considered during toolchain
    /// resolution.
    pub fn get_extra_toolchains(&self) -> &[String] {
        &self.extra_toolchains
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:105 PlatformConfiguration.debugToolchainResolution
    /// Returns true if toolchain resolution debug info should be printed for this label, which
    /// could be a toolchain type or a specific target.
    // PORT: Java overloads `debugToolchainResolution(Label)`; Rust names the single-label form.
    pub fn debug_toolchain_resolution_for_label(&self, label: &Label) -> bool {
        self.debug_toolchain_resolution(std::slice::from_ref(label))
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:105 PlatformConfiguration.debugToolchainResolution
    /// Returns true if toolchain resolution debug info should be printed for any of these labels,
    /// which could be either toolchain types or specific targets.
    pub fn debug_toolchain_resolution(&self, labels: &[Label]) -> bool {
        if labels.is_empty() {
            // Check an empty string, in case the filter is .*
            return self.toolchain_resolution_debug_regex_filter.test("");
        }
        labels
            .iter()
            .map(Label::get_canonical_form)
            .any(|canonical| {
                self.toolchain_resolution_debug_regex_filter
                    .test(&canonical)
            })
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:134 PlatformConfiguration.toolchainResolutionDebugFilter
    /// Returns a value for `--toolchain_resolution_debug` that matches exactly the given label,
    /// for use in error messages.
    ///
    /// The filter is matched against `Label::get_canonical_form`, so the canonical form is used
    /// even if the label has a shorter display form.
    pub fn toolchain_resolution_debug_filter(label: &Label) -> String {
        let canonical_form = label.get_canonical_form();
        for c in REGEX_SPECIAL_CHARS {
            if canonical_form.contains(*c) {
                return pattern_quote(&canonical_form);
            }
        }
        canonical_form
    }
}

// PORT: JDK `java.util.regex.Pattern.quote`, ported locally.
/// Returns a literal pattern String for the specified String.
fn pattern_quote(s: &str) -> String {
    if !s.contains("\\E") {
        return format!("\\Q{}\\E", s);
    }
    let mut sb = String::with_capacity(s.len() * 2);
    sb.push_str("\\Q");
    let mut current = 0;
    while let Some(i) = s[current..].find("\\E") {
        let slash_e_index = current + i;
        sb.push_str(&s[current..slash_e_index]);
        current = slash_e_index + 2;
        sb.push_str("\\E\\\\E\\Q");
    }
    sb.push_str(&s[current..]);
    sb.push_str("\\E");
    sb
}

impl Fragment for PlatformConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &PLATFORM_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:53 PlatformConfiguration.reportInvalidOptions
    fn report_invalid_options(&self, reporter: &dyn EventHandler, build_options: &BuildOptions) {
        let platform_options = required::<PlatformOptions>(build_options);
        // TODO(https://github.com/bazelbuild/bazel/issues/6519): Implement true multiplatform
        // builds.
        if platform_options.platforms.len() > 1 {
            reporter.handle(&Event::warn(format!(
                "--platforms only supports a single target platform: using the first option {}",
                self.target_platform
            )));
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_quote_matches_jdk() {
        assert_eq!(pattern_quote("a.b"), "\\Qa.b\\E");
        assert_eq!(pattern_quote("a\\Eb"), "\\Qa\\E\\\\E\\Qb\\E");
    }

    #[test]
    fn toolchain_resolution_debug_filter_quotes_special_chars() {
        let plain = Label::parse_canonical_unchecked("//foo:bar");
        assert_eq!(
            PlatformConfiguration::toolchain_resolution_debug_filter(&plain),
            "//foo:bar"
        );
        let special = Label::parse_canonical_unchecked("//foo:bar.baz");
        assert_eq!(
            PlatformConfiguration::toolchain_resolution_debug_filter(&special),
            "\\Q//foo:bar.baz\\E"
        );
    }
}
