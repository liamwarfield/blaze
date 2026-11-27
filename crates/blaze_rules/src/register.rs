//! Registers the native configured target factories (Java: each rule set's `init` adding rule
//! classes with their factories to the `ConfiguredRuleClassProvider`).

use blaze_analysis::configured_rule_class_provider::{
    ConfiguredRuleClassProvider, ConfiguredRuleClassProviderBuilder,
};

/// Adds every native factory ported so far.
pub fn register_factories(builder: &mut ConfiguredRuleClassProviderBuilder) {
    crate::platform::register(builder);
    crate::config::register(builder);
    crate::generic::register(builder);
    crate::genquery::register(builder);
    crate::java::register(builder);
    crate::extra::register(builder);
    crate::rules_core::register(builder);
    crate::starlarkdocextract::register(builder);
}

/// Bazel's analysis rule class provider.
pub fn create_bazel_rule_class_provider() -> ConfiguredRuleClassProvider {
    let mut builder = ConfiguredRuleClassProvider::builder();
    register_factories(&mut builder);
    builder.build()
}

/// Installs the process-wide analysis hooks and registrations (Java: done by the rule class
/// provider's construction): the subrule call, the fragments' Starlark values.
pub fn install_analysis_hooks() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        blaze_analysis::starlark::install_hooks();
        blaze_analysis::starlark::starlark_action_factory::set_transform_build_info_file(
            crate::build_info::transform_build_info_file_deferred,
        );
        crate::fragments::register_fragments();
        crate::cpp::register_fragments();
    });
}
