//! Ports of `rules/test/TestSuite.java` and `rules/test/TransitiveTestsProvider.java`.

use std::collections::HashSet;

use blaze_actions::analysis::runfiles::{Runfiles, RunfilesBuilder};
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::provider_map::TransitiveInfoProvider;
use blaze_analysis::providers::file_provider::{RunfilesMapping, RunfilesProvider};
use blaze_analysis::rule::config_util;
use blaze_analysis::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_analysis::rule::runfiles_util;
use blaze_analysis::test::test_provider::TestTagsProvider;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::target::target_utils;
use blaze_packages::types::Type;

/// A `TransitiveInfoProvider` for configured targets to implement test_suite expansion.
// Java: com/google/devtools/build/lib/rules/test/TransitiveTestsProvider.java:21 TransitiveTestsProvider
#[derive(Clone, Debug)]
pub struct TransitiveTestsProvider;

impl TransitiveInfoProvider for TransitiveTestsProvider {
    fn java_class_name(&self) -> &'static str {
        "TransitiveTestsProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Implementation for the "test_suite" rule.
// Java: com/google/devtools/build/lib/rules/test/TestSuite.java:40 TestSuite
pub struct TestSuite;

impl RuleConfiguredTargetFactory for TestSuite {
    // Java: com/google/devtools/build/lib/rules/test/TestSuite.java:43 TestSuite.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        Self::check_tests_and_suites(rule_context, "tests");
        if rule_context.has_errors() {
            return Ok(None);
        }

        // CAUTION: The rules for what goes into and what comes out of a test_suite are
        // complicated. The specification is in BuildEncyclopedia.
        let mut tags_attribute: Vec<String> = rule_context
            .attributes()
            .get("tags", Type::StringList)
            .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
            .unwrap_or_default();
        // Java: tagsAttribute.remove("manual") removes the first occurrence.
        if let Some(i) = tags_attribute.iter().position(|t| t == "manual") {
            tags_attribute.remove(i);
        }
        let (required, excluded) = target_utils::sort_tags_by_sense(&tags_attribute);

        let mut direct_tests_and_suites: Vec<ConfiguredTarget> = Vec::new();
        let mut deps = Self::get_prerequisites(rule_context, "tests");
        deps.extend(Self::get_prerequisites(rule_context, "$implicit_tests"));
        for dep in deps {
            if let Some(tags_provider) =
                blaze_analysis::configuredtargets::get_provider::<TestTagsProvider>(&dep)
            {
                let tags: HashSet<String> = tags_provider.get_test_tags().iter().cloned().collect();
                if !target_utils::test_matches_filters(&tags, &required, &excluded, true) {
                    continue;
                }
            }
            direct_tests_and_suites.push(dep);
        }

        let mut runfiles_builder = RunfilesBuilder::new(rule_context.get_workspace_name());
        runfiles_util::add_targets(
            &mut runfiles_builder,
            &direct_tests_and_suites,
            RunfilesMapping::DataRunfiles,
            config_util::always_include_files_to_build_in_data(rule_context.get_configuration()),
        );
        let runfiles = runfiles_builder.build();

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::with_data(Runfiles::empty(), runfiles))
            .add_provider(TransitiveTestsProvider);
        match builder.build() {
            Ok(t) => Ok(t),
            Err(RuleConfiguredTargetBuildError::ActionConflict(m)) => {
                Err(CreateConfiguredTargetError::ActionConflict(m))
            }
            Err(RuleConfiguredTargetBuildError::Interrupted) => {
                Err(CreateConfiguredTargetError::Interrupted)
            }
        }
    }
}

impl TestSuite {
    // Java: com/google/devtools/build/lib/rules/test/TestSuite.java:93 TestSuite.getPrerequisites
    fn get_prerequisites(
        rule_context: &RuleContext,
        attribute_name: &str,
    ) -> Vec<ConfiguredTarget> {
        if rule_context
            .attributes()
            .has_typed(attribute_name, Type::LabelList)
        {
            rule_context.get_prerequisites(attribute_name)
        } else {
            Vec::new()
        }
    }

    // Java: com/google/devtools/build/lib/rules/test/TestSuite.java:102 TestSuite.checkTestsAndSuites
    fn check_tests_and_suites(rule_context: &RuleContext, attribute_name: &str) {
        if !rule_context
            .attributes()
            .has_typed(attribute_name, Type::LabelList)
        {
            return;
        }
        for dep in rule_context.get_prerequisites(attribute_name) {
            // Okay to check for TransitiveTestsProvider instead of TestSuite because test_suite
            // rule is the only one that provides it.
            let provider =
                blaze_analysis::configuredtargets::get_provider::<TransitiveTestsProvider>(&dep);
            let tags_provider =
                blaze_analysis::configuredtargets::get_provider::<TestTagsProvider>(&dep);
            if provider.is_none() && tags_provider.is_none() {
                rule_context.attribute_error(
                    attribute_name,
                    &format!(
                        "expecting a test or a test_suite rule but '{}' is not one",
                        blaze_analysis::configuredtargets::get_label(&dep)
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/rules/test/TestSuiteTest.java.
    #[test]
    #[ignore = "needs analysis of BUILD files (TestSuiteTest.testTestSuiteWithNonTestTarget)"]
    fn test_test_suite_with_non_test_target() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (TestSuiteTest.testTestSuiteRunfiles)"]
    fn test_test_suite_runfiles() {}
}
