//! Port of `rules/genquery`: the `genquery` rule (`GenQuery`), its scope traversal
//! (`GenQueryPackageProviderFactory`, GENQUERY_SCOPE), package provider and output stream.

pub mod gen_query;
pub mod gen_query_output_stream;
pub mod gen_query_package_provider;

pub use gen_query::{GenQuery, QueryResultAction};

/// Registers the factory (Java `GenQueryRule.getMetadata().factoryClass(GenQuery.class)`).
pub fn register(
    builder: &mut blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder,
) {
    gen_query::register_factory(builder);
}

/// Registers GENQUERY_SCOPE.
pub fn register_sky_functions(
    builder: blaze_skyframe::EvaluatorBuilder,
) -> blaze_skyframe::EvaluatorBuilder {
    gen_query_package_provider::register(builder)
}
