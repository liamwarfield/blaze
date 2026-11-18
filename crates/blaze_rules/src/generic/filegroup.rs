//! Port of `rules/filegroup/Filegroup.java`.

use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles::RunfilesBuilder;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::downcast_info;
use blaze_analysis::providers::file_provider::{RunfilesMapping, RunfilesProvider};
use blaze_analysis::providers::output_group_info::{INTERNAL_SUFFIX, OutputGroupInfo};
use blaze_analysis::rule::config_util;
use blaze_analysis::rule::prerequisite_artifacts::PrerequisiteArtifacts;
use blaze_analysis::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_analysis::rule::runfiles_util;
use blaze_analysis::test::instrumented_files_info::{
    InstrumentationSpec, InstrumentedFilesCollector,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO;
use blaze_packages::types::{FileTypeSet, Type};

/// Error message for output groups that are explicitly blacklisted for filegroup reference.
// Java: com/google/devtools/build/lib/rules/filegroup/Filegroup.java:46 Filegroup.ILLEGAL_OUTPUT_GROUP_ERROR
pub fn illegal_output_group_error(output_group_name: &str) -> String {
    format!("Output group {output_group_name} is not permitted for reference in filegroups.")
}

/// ConfiguredTarget for "filegroup".
// Java: com/google/devtools/build/lib/rules/filegroup/Filegroup.java:43 Filegroup
pub struct Filegroup;

impl RuleConfiguredTargetFactory for Filegroup {
    // Java: com/google/devtools/build/lib/rules/filegroup/Filegroup.java:50 Filegroup.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        let output_group_name = rule_context
            .attributes()
            .get("output_group", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if output_group_name.ends_with(INTERNAL_SUFFIX) {
            return Err(rule_context
                .throw_with_attribute_error(
                    "output_group",
                    &illegal_output_group_error(&output_group_name),
                )
                .into());
        }

        let files_to_build: NestedSet<Artifact> = if output_group_name.is_empty() {
            PrerequisiteArtifacts::nested_set(
                rule_context.get_rule_prerequisites_collection(),
                "srcs",
            )
        } else {
            Self::get_artifacts_for_output_group(
                &output_group_name,
                &rule_context.get_prerequisites("srcs"),
            )
        };

        let instrumented_files_provider = InstrumentedFilesCollector::collect(
            rule_context,
            &InstrumentationSpec::new(FileTypeSet::AnyFile)
                .with_dependency_attributes(&["srcs", "data"]),
            /* reported_to_actual_sources= */
            NestedSetBuilder::create(Order::STABLE_ORDER, std::iter::empty()),
        );

        let mut data_runfiles_builder = RunfilesBuilder::new(rule_context.get_workspace_name());
        data_runfiles_builder.add_transitive_artifacts(&files_to_build);
        if config_util::core_options(rule_context.get_configuration()).filegroup_runfiles_for_data {
            runfiles_util::add_runfiles(
                &mut data_runfiles_builder,
                rule_context,
                RunfilesMapping::DataRunfiles,
            );
        } else {
            runfiles_util::add_data_deps(&mut data_runfiles_builder, rule_context);
        }
        let mut default_runfiles_builder = RunfilesBuilder::new(rule_context.get_workspace_name());
        runfiles_util::add_runfiles(
            &mut default_runfiles_builder,
            rule_context,
            RunfilesMapping::DefaultRunfiles,
        );
        let runfiles_provider = RunfilesProvider::with_data(
            default_runfiles_builder.build(),
            data_runfiles_builder.build(),
        );

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(runfiles_provider)
            .set_files_to_build(files_to_build.clone())
            .set_runfiles_support(None, Self::get_executable(&files_to_build))
            .add_native_declared_provider(instrumented_files_provider);

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

impl Filegroup {
    // Java: com/google/devtools/build/lib/rules/filegroup/Filegroup.java:100 Filegroup.getExecutable
    fn get_executable(files_to_build: &NestedSet<Artifact>) -> Option<Artifact> {
        if files_to_build.is_singleton() {
            Some(files_to_build.get_singleton().clone())
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/rules/filegroup/Filegroup.java:104 Filegroup.getArtifactsForOutputGroup
    fn get_artifacts_for_output_group(
        output_group_name: &str,
        deps: &[ConfiguredTarget],
    ) -> NestedSet<Artifact> {
        let mut result: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        for dep in deps {
            // Java: OutputGroupInfo.get(dep).
            if let Some(info) =
                blaze_analysis::configuredtargets::get_info(dep, &OUTPUT_GROUP_INFO.key())
            {
                let output_group_info =
                    downcast_info::<OutputGroupInfo>(&info).expect("OutputGroupInfo");
                result.add_transitive(&output_group_info.get_output_group(output_group_name));
            }
        }
        result.build()
    }
}

/// Keeps `AttributeMap` in scope for trait method calls.
#[allow(unused)]
fn _uses(_: &dyn AttributeMap) {}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/rules/filegroup/FilegroupConfiguredTargetTest.java.
    use super::*;

    #[test]
    fn illegal_output_group_error_text() {
        assert_eq!(
            illegal_output_group_error("_hidden_top_level_INTERNAL_"),
            "Output group _hidden_top_level_INTERNAL_ is not permitted for reference in filegroups."
        );
    }

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testGroup)"]
    fn test_group() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testDependencyGraph)"]
    fn test_dependency_graph() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testEmptyGroupInGenrule)"]
    fn test_empty_group_in_genrule() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testIsNotExecutable)"]
    fn test_is_not_executable() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testIsExecutable)"]
    fn test_is_executable() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testOutputGroupExtractsFilesFromDependencies)"]
    fn test_output_group_extracts_files_from_dependencies() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (FilegroupConfiguredTargetTest.testErrorForIllegalOutputGroup)"]
    fn test_error_for_illegal_output_group() {}
}
