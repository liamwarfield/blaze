//! The `Runfiles.Builder` methods that read a `RuleContext` or `TransitiveInfoCollection`s
//! (Java `analysis/Runfiles.java`). `blaze_actions::analysis::runfiles::RunfilesBuilder` ports the
//! rest of the builder; these need the analysis types, so they live here as functions over it.

use blaze_actions::analysis::runfiles::{Runfiles, RunfilesBuilder};
use blaze_collect::NestedSetBuilder;
use blaze_packages::target::RuleOrMacroInstance;
use blaze_packages::types::Type;

use crate::configured_target::ConfiguredTarget;
use crate::providers::file_provider::{FileProvider, RunfilesMapping, RunfilesProvider};
use crate::rule::config_util;
use crate::rule::rule_context::RuleContext;

/// Applies `RunfilesProvider.DEFAULT_RUNFILES` / `DATA_RUNFILES` to a target.
// Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:47 RunfilesProvider.DEFAULT_RUNFILES
// Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:63 RunfilesProvider.DATA_RUNFILES
pub fn apply_mapping(mapping: RunfilesMapping, input: &ConfiguredTarget) -> Runfiles {
    match crate::configuredtargets::get_provider::<RunfilesProvider>(input) {
        Some(provider) => match mapping {
            RunfilesMapping::DefaultRunfiles => provider.get_default_runfiles().clone(),
            RunfilesMapping::DataRunfiles => provider.get_data_runfiles().clone(),
        },
        None => Runfiles::empty(),
    }
}

/// Adds the runfiles for a particular target and visits the transitive closure of "srcs",
/// "deps" and "data", collecting all of their respective runfiles.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:669 Runfiles.Builder.addRunfiles
pub fn add_runfiles(
    builder: &mut RunfilesBuilder,
    rule_context: &RuleContext,
    mapping: RunfilesMapping,
) {
    add_data_deps(builder, rule_context);
    add_non_data_deps(builder, rule_context, mapping);
}

/// Adds the files specified by a mapping from the transitive info collection to the runfiles.
/// Dependencies in `srcs` and `deps` are considered.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:684 Runfiles.Builder.add
pub fn add(builder: &mut RunfilesBuilder, rule_context: &RuleContext, mapping: RunfilesMapping) {
    for dep in get_non_data_deps(rule_context) {
        let runfiles = apply_mapping(mapping, &dep);
        builder.merge(&runfiles);
    }
}

/// Collects runfiles from data dependencies of a target.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:700 Runfiles.Builder.addDataDeps
pub fn add_data_deps(builder: &mut RunfilesBuilder, rule_context: &RuleContext) {
    add_targets(
        builder,
        &get_prerequisites(rule_context, "data"),
        RunfilesMapping::DataRunfiles,
        config_util::always_include_files_to_build_in_data(rule_context.get_configuration()),
    );
}

/// Collects runfiles from "srcs" and "deps" of a target.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:710 Runfiles.Builder.addNonDataDeps
pub fn add_non_data_deps(
    builder: &mut RunfilesBuilder,
    rule_context: &RuleContext,
    mapping: RunfilesMapping,
) {
    for target in get_non_data_deps(rule_context) {
        add_target_except_file_targets(builder, &target, mapping);
    }
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:719 Runfiles.Builder.addTargets
pub fn add_targets(
    builder: &mut RunfilesBuilder,
    targets: &[ConfiguredTarget],
    mapping: RunfilesMapping,
    always_include_files_to_build_in_data: bool,
) {
    for target in targets {
        add_target(
            builder,
            target,
            mapping,
            always_include_files_to_build_in_data,
        );
    }
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:730 Runfiles.Builder.addTarget
pub fn add_target(
    builder: &mut RunfilesBuilder,
    target: &ConfiguredTarget,
    mapping: RunfilesMapping,
    always_include_files_to_build_in_data: bool,
) {
    add_target_including_file_targets(
        builder,
        target,
        mapping,
        always_include_files_to_build_in_data,
    );
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:738 Runfiles.Builder.addTargetExceptFileTargets
fn add_target_except_file_targets(
    builder: &mut RunfilesBuilder,
    target: &ConfiguredTarget,
    mapping: RunfilesMapping,
) {
    let runfiles = apply_mapping(mapping, target);
    builder.merge(&runfiles);
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:748 Runfiles.Builder.addTargetIncludingFileTargets
fn add_target_including_file_targets(
    builder: &mut RunfilesBuilder,
    target: &ConfiguredTarget,
    mapping: RunfilesMapping,
    always_include_files_to_build_in_data: bool,
) {
    if crate::configuredtargets::get_provider::<RunfilesProvider>(target).is_none()
        && mapping == RunfilesMapping::DataRunfiles
    {
        // RuleConfiguredTarget implements RunfilesProvider, so this will only be called on
        // FileConfiguredTarget instances.
        // TODO(bazel-team): This is a terrible hack. We should be able to make this go away
        // by implementing RunfilesProvider on FileConfiguredTarget. We'd need to be mindful
        // of the memory use, though, since we have a whole lot of FileConfiguredTarget
        // instances.
        builder.add_transitive_artifacts(
            crate::configuredtargets::get_provider::<FileProvider>(target)
                .expect("FileProvider")
                .get_files_to_build(),
        );
        return;
    }

    if always_include_files_to_build_in_data && mapping == RunfilesMapping::DataRunfiles {
        // Ensure that `DefaultInfo.files` of Starlark rules is merged in so that native rules
        // interoperate well with idiomatic Starlark rules..
        // https://bazel.build/extending/rules#runfiles_features_to_avoid
        // Internal tests fail if the order of filesToBuild is preserved.
        let mut b = NestedSetBuilder::stable_order();
        b.add_transitive(
            crate::configuredtargets::get_provider::<FileProvider>(target)
                .expect("FileProvider")
                .get_files_to_build(),
        );
        builder.add_transitive_artifacts(&b.build());
    }

    add_target_except_file_targets(builder, target, mapping);
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:777 Runfiles.Builder.getNonDataDeps
fn get_non_data_deps(rule_context: &RuleContext) -> Vec<ConfiguredTarget> {
    // TODO(bazel-team): This line shouldn't be here. Removing it requires that no rules have
    // dependent rules in srcs (except for filegroups and such), but always in deps.
    // TODO(bazel-team): DONT_CHECK is not optimal here. Rules that use split configs need to
    // be changed not to call into here.
    let mut v = get_prerequisites(rule_context, "srcs");
    v.extend(get_prerequisites(rule_context, "deps"));
    v
}

/// For the specified attribute (which must be of type list(label)), resolves all the labels
/// into ConfiguredTargets (for the same configuration as this one) and returns them as a list.
/// If the rule does not have the specified attribute, returns the empty list.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:794 Runfiles.Builder.getPrerequisites
fn get_prerequisites(rule_context: &RuleContext, attribute_name: &str) -> Vec<ConfiguredTarget> {
    if rule_context
        .get_rule()
        .is_attr_defined(attribute_name, Type::LabelList)
    {
        rule_context.get_prerequisites(attribute_name)
    } else {
        Vec::new()
    }
}
