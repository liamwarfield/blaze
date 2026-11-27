//! The `starlark_doc_extract` rule (Java `rules/starlarkdocextract`).

pub mod starlark_doc_extract;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// Registers the configured target factory of `starlark_doc_extract`.
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder.add_factory(
        "StarlarkDocExtract",
        std::sync::Arc::new(starlark_doc_extract::StarlarkDocExtract),
    );
}
