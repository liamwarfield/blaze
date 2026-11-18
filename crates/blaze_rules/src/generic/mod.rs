//! Generic native rules (agent R): `filegroup` (`rules/filegroup/Filegroup.java`), `test_suite`
//! (`rules/test/TestSuite.java`) and `genrule` (`rules/genrule/GenRuleBase.java`,
//! `bazel/rules/genrule/BazelGenRule.java`).

pub mod filegroup;
pub mod genrule;
pub mod target_utils;
pub mod test_suite;

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// Registers the generic rules' configured target factories under their Java factory class
/// simple names (the names `blaze_packages::native_rules::generic_rules` records).
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder.add_factory("Filegroup", Arc::new(filegroup::Filegroup));
    builder.add_factory("TestSuite", Arc::new(test_suite::TestSuite));
    // Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:51 BazelGenRuleRule.getMetadata
    builder.add_factory("BazelGenRule", Arc::new(genrule::BazelGenRule));
}
