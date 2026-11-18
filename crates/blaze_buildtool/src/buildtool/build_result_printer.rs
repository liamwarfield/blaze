//! Port of `buildtool/BuildResultPrinter.java` (`--show_result`), with the parts of
//! `analysis/TopLevelArtifactHelper.java` and `analysis/TopLevelArtifactContext.java` it needs and
//! the target-count messages of `buildtool/AnalysisPhaseRunner.java`.

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_collect::nestedset::NestedSetBuilder;
use blaze_events::{Event, EventHandler, OutErr};
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::build_result::{BuildResult, ConfiguredTargetRef, contains_target};
use super::output_directory_links_utils::get_symlink_prefix;
use super::path_pretty_printer::PathPrettyPrinter;

/// `OutputGroupInfo.DEFAULT`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:148 OutputGroupInfo.DEFAULT
pub const DEFAULT: &str = "default";
/// `OutputGroupInfo.HIDDEN_OUTPUT_GROUP_PREFIX`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:85 OutputGroupInfo.HIDDEN_OUTPUT_GROUP_PREFIX
pub const HIDDEN_OUTPUT_GROUP_PREFIX: &str = "_";
/// `OutputGroupInfo.TEMP_FILES`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:145 OutputGroupInfo.TEMP_FILES
pub const TEMP_FILES: &str = "temp_files_INTERNAL_";
/// `AspectCollection.VALIDATION_ASPECT_NAME`.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:153 AspectCollection.VALIDATION_ASPECT_NAME
pub const VALIDATION_ASPECT_NAME: &str = "ValidateTarget";

/// Contains options which control the set of artifacts to build for top-level targets.
///
/// PORT: only `outputGroups` (the value of `--output_groups`, sorted) is kept; the other
/// components (`runTestsExclusively`, `expandFilesets`, `fullyResolveFilesetSymlinks`) are not
/// read by the ported code.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactContext.java:27 TopLevelArtifactContext
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TopLevelArtifactContext {
    /// `outputGroups()`: an `ImmutableSortedSet<String>`.
    pub output_groups: BTreeSet<String>,
}

/// A configured aspect as `BuildResultPrinter` reads it, with the parts of its `AspectKey`.
///
/// PORT: neither `AspectKey` nor `ConfiguredAspect` is ported; the analysis crates implement this
/// view (the key itself is an [`ActionLookupKeyRef`]).
pub trait ConfiguredAspectView: Send + Sync {
    /// `AspectKey.getLabel()`.
    fn get_label(&self) -> Label;
    /// `AspectKey.getAspectClass().getName()`.
    fn get_aspect_class_name(&self) -> String;
    /// `AspectKey.getBaseConfiguredTargetKey()`.
    fn get_base_configured_target_key(&self) -> ActionLookupKeyRef;
    /// The aspect's `OutputGroupInfo`, in insertion order.
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>>;
}

/// What `BuildResultPrinter.filterTargetsToPrint` reads from an `OutputFileConfiguredTarget` and
/// its generating rule.
///
/// PORT: the [`ConfiguredTargetRef`] view does not reach `OutputFileConfiguredTarget
/// .getGeneratingRule()` or the `FileProvider`s; the command, which holds the analysis result's
/// configured targets, answers for a `generated file` target (see [`GeneratingRuleLookup`]).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:69 OutputFileConfiguredTarget.getGeneratingRule
pub struct OutputFileGeneratingRule {
    /// `ConfiguredTargetKey.fromConfiguredTarget(getGeneratingRule())`: stands for the generating
    /// rule in `configuredTargets.contains(generatingRule)`.
    pub generating_rule_key: ActionLookupKeyRef,
    /// `generatingRule.getProvider(FileProvider.class).getFilesToBuild()`.
    pub generating_rule_files_to_build: NestedSet<Artifact>,
    /// `configuredTarget.getProvider(FileProvider.class).getFilesToBuild()`.
    pub files_to_build: NestedSet<Artifact>,
}

/// Answers [`OutputFileGeneratingRule`] for a target whose kind is `generated file`.
pub type GeneratingRuleLookup<'a> = &'a dyn Fn(&ConfiguredTargetRef) -> OutputFileGeneratingRule;

/// What `BuildResultPrinter` reads from the `BuildRequest`.
///
/// PORT: `BuildRequest` is the build driver's; it implements this view.
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:59 BuildRequest
pub trait BuildRequestView {
    /// `getBuildOptions()`.
    fn get_build_options(&self) -> &BuildRequestOptions;
    /// `getOptions(ExecutionOptions.class)`.
    fn get_execution_options(&self) -> &ExecutionOptions;
    /// `useValidationAspect()`.
    fn use_validation_aspect(&self) -> bool;
    /// `getTopLevelArtifactContext()`.
    fn get_top_level_artifact_context(&self) -> TopLevelArtifactContext;
    /// `getOutErr()`.
    fn get_out_err(&self) -> &OutErr;
}

/// `TopLevelArtifactHelper.getAllArtifactsToBuild(target, context).getImportantArtifacts()`.
///
/// `output_groups` is the target's `OutputGroupInfo`; for a configured target its `default` group
/// already holds `FileProvider.getFilesToBuild()` (see `ConfiguredTargetView.get_output_groups`),
/// so `files_to_build` is `None` here.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:204 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn get_important_artifacts(
    output_groups: &IndexMap<String, NestedSet<Artifact>>,
    files_to_build: Option<&NestedSet<Artifact>>,
    context: &TopLevelArtifactContext,
) -> NestedSet<Artifact> {
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:97 ArtifactsToBuild.getImportantArtifacts
    let mut important = NestedSetBuilder::stable_order();
    for output_group in &context.output_groups {
        let mut results = NestedSetBuilder::stable_order();

        if output_group == DEFAULT
            && let Some(files_to_build) = files_to_build
        {
            results.add_transitive(files_to_build);
        }

        if let Some(group) = output_groups.get(output_group) {
            results.add_transitive(group);
        }

        // Ignore output groups that have no artifacts.
        if results.is_empty() {
            continue;
        }

        let is_important_group = !output_group.starts_with(HIDDEN_OUTPUT_GROUP_PREFIX);
        if is_important_group {
            important.add_transitive(&results.build());
        }
    }
    important.build()
}

/// Returns false if the build outputs provided by the target should never be shown to users.
///
/// Always returns false for hidden rules and source file targets.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:268 TopLevelArtifactHelper.shouldConsiderForDisplay
pub fn should_consider_for_display(configured_target: &ConfiguredTargetRef) -> bool {
    // TODO(bazel-team): this is quite ugly. Add a marker provider for this check.
    let kind = configured_target.get_rule_class_or_kind();
    if kind == "source file" {
        // Suppress display of source files (because we do no work to build them).
        return false;
    }
    if kind != "generated file" && kind.contains('$') {
        // Suppress display of hidden rules
        return false;
    }
    true
}

/// Returns true if the given artifact should be shown to users as a build output.
///
/// Always returns false for runfiles tree and source artifacts.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:288 TopLevelArtifactHelper.shouldDisplay
pub fn should_display(artifact: &Artifact) -> bool {
    !artifact.is_source_artifact() && !artifact.is_runfiles_tree()
}

/// Reports the number of targets and test targets found by the analysis phase.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:492 AnalysisPhaseRunner.reportTargetsWithTests
pub fn report_targets_with_tests(
    reporter: &dyn EventHandler,
    targets_to_build: &[ConfiguredTargetRef],
    targets_to_test: &[ConfiguredTargetRef],
) {
    let test_count = targets_to_test.len() as i32;
    let target_count = targets_to_build.len() as i32 - test_count;
    if target_count == 0 {
        reporter.handle(&Event::info(format!(
            "Found {}{}",
            test_count,
            if test_count == 1 {
                " test target..."
            } else {
                " test targets..."
            }
        )));
    } else {
        reporter.handle(&Event::info(format!(
            "Found {}{}{}{}",
            target_count,
            if target_count == 1 {
                " target and "
            } else {
                " targets and "
            },
            test_count,
            if test_count == 1 {
                " test target..."
            } else {
                " test targets..."
            }
        )));
    }
}

/// Reports the number of targets found by the analysis phase.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:518 AnalysisPhaseRunner.reportTargets
pub fn report_targets(reporter: &dyn EventHandler, targets_to_build: &[ConfiguredTargetRef]) {
    let target_count = targets_to_build.len();
    reporter.handle(&Event::info(format!(
        "Found {}{}",
        target_count,
        if target_count == 1 {
            " target..."
        } else {
            " targets..."
        }
    )));
}

/// Handles --show_result and --experimental_show_artifacts.
///
/// PORT: Java holds the `CommandEnvironment`; the printer reads only the product name
/// (`env.getRuntime().getProductName()`) and `env.getRelativeWorkingDirectory()`.
// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:46 BuildResultPrinter
pub struct BuildResultPrinter {
    product_name: String,
    relative_working_directory: PathFragment,
}

impl BuildResultPrinter {
    // Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:49 BuildResultPrinter.<init>
    pub fn new(product_name: &str, relative_working_directory: PathFragment) -> BuildResultPrinter {
        BuildResultPrinter {
            product_name: product_name.to_string(),
            relative_working_directory,
        }
    }

    /// Shows the result of the build. Information includes the list of up-to-date and failed
    /// targets and list of output artifacts for successful targets
    ///
    /// This corresponds to the --show_result flag.
    // Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:59 BuildResultPrinter.showBuildResult
    pub fn show_build_result(
        &self,
        request: &dyn BuildRequestView,
        result: &BuildResult,
        configured_targets: &[ConfiguredTargetRef],
        configured_targets_to_skip: &[ConfiguredTargetRef],
        aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
        generating_rule_lookup: GeneratingRuleLookup<'_>,
    ) {
        // NOTE: be careful what you print!  We don't want to create a consistency
        // problem where the summary message and the exit code disagree.  The logic
        // here is already complex.
        let ok = self.output_targets(
            request,
            result,
            configured_targets,
            configured_targets_to_skip,
            aspects,
            generating_rule_lookup,
        );
        if !ok && !request.get_execution_options().verbose_failures {
            request.get_out_err().print_err(
                "Use --verbose_failures to see the command lines of failed build steps.\n",
            );
        }
    }

    /// Outputs the targets, omitting values with `(nothing to build)` when it allows staying
    /// under the --show_result limit.
    ///
    /// This method exits early if there are too many results.
    ///
    /// Returns `true` if no errors were detected among the results inspected, this can be a false
    /// positive on early exit.
    // Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:86 BuildResultPrinter.outputTargets
    fn output_targets(
        &self,
        request: &dyn BuildRequestView,
        result: &BuildResult,
        configured_targets: &[ConfiguredTargetRef],
        configured_targets_to_skip: &[ConfiguredTargetRef],
        aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
        generating_rule_lookup: GeneratingRuleLookup<'_>,
    ) -> bool {
        let product_name = &self.product_name;
        let pretty_printer = PathPrettyPrinter::new(
            &self.relative_working_directory,
            &get_symlink_prefix(request.get_build_options(), product_name),
            result.get_convenience_symlinks().clone(),
        );
        let out_err = request.get_out_err();

        // Filter and split aspects to display.
        let aspects_to_ignore: IndexSet<&String> = request
            .get_build_options()
            .hide_aspect_results
            .iter()
            .collect();
        let partitioned_aspect_keys = partition_aspect_keys(
            request.use_validation_aspect(),
            aspects
                .iter()
                .filter(|(_, a)| !aspects_to_ignore.contains(&a.get_aspect_class_name()))
                .map(|(k, _)| k.clone())
                .collect(),
            aspects,
        );

        let targets_to_print = filter_targets_to_print(configured_targets, generating_rule_lookup);
        let context = request.get_top_level_artifact_context();

        // `essentialBudget` tracks the number of non-empty results that can be printed.
        let mut essential_budget = request.get_build_options().max_result_targets;

        // Splits the targets we care about into three buckets. Targets are only considered
        // successful if they and their validation aspects succeeded.
        let mut skipped: Vec<ConfiguredTargetRef> = Vec::new();
        let mut succeeded: Vec<ConfiguredTargetRef> = Vec::new();
        let mut artifacts_to_print_per_target: Vec<Vec<Artifact>> = Vec::new();
        let mut failed: Vec<ConfiguredTargetRef> = Vec::new();
        essential_budget = split_configured_targets_by_result_return_remaining(
            &targets_to_print,
            result,
            &context,
            configured_targets_to_skip,
            &partitioned_aspect_keys.validation_aspects,
            aspects,
            &mut skipped,
            &mut succeeded,
            &mut artifacts_to_print_per_target,
            &mut failed,
            essential_budget,
        );
        if essential_budget < 0 {
            return failed.is_empty();
        }

        // Splits the aspects we care about into two buckets.
        let mut successful_aspects: Vec<ActionLookupKeyRef> = Vec::new();
        let mut failed_aspects: Vec<ActionLookupKeyRef> = Vec::new();
        let mut artifacts_to_print_per_aspect: Vec<Vec<Artifact>> = Vec::new();
        essential_budget = split_aspects_by_result_return_remaining(
            &partitioned_aspect_keys.aspects_to_print,
            aspects,
            &context,
            result.get_successful_aspects(),
            &mut successful_aspects,
            &mut artifacts_to_print_per_aspect,
            &mut failed_aspects,
            essential_budget,
        );
        if essential_budget < 0 {
            return failed.is_empty() && failed_aspects.is_empty();
        }

        // Omits "nothing to build" values if it enables staying under --show_result.
        let omit_nothing_to_build =
            (targets_to_print.len() + partitioned_aspect_keys.aspects_to_print.len()) as i64
                > request.get_build_options().max_result_targets as i64;

        output_configured_targets(
            out_err,
            &pretty_printer,
            &succeeded,
            &artifacts_to_print_per_target,
            &failed,
            &skipped,
            omit_nothing_to_build,
        );
        output_aspects(
            out_err,
            &pretty_printer,
            &successful_aspects,
            aspects,
            &artifacts_to_print_per_aspect,
            &failed_aspects,
            omit_nothing_to_build,
        );

        failed.is_empty() && failed_aspects.is_empty()
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:181 BuildResultPrinter.splitConfiguredTargetsByResultReturnRemaining
fn split_configured_targets_by_result_return_remaining(
    configured_targets: &[ConfiguredTargetRef],
    result: &BuildResult,
    context: &TopLevelArtifactContext,
    configured_targets_to_skip: &[ConfiguredTargetRef],
    validation_aspects: &[ActionLookupKeyRef],
    aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
    skipped: &mut Vec<ConfiguredTargetRef>,
    succeeded: &mut Vec<ConfiguredTargetRef>,
    artifacts_to_print_per_target: &mut Vec<Vec<Artifact>>,
    failed: &mut Vec<ConfiguredTargetRef>,
    mut essential_budget: i32,
) -> i32 {
    let validation_failures: IndexSet<ActionLookupKeyRef> = validation_aspects
        .iter()
        .filter(|k| {
            !result
                .get_successful_aspects()
                .is_some_and(|s| s.contains(*k))
        })
        .map(|k| aspects.get(k).unwrap().get_base_configured_target_key())
        .collect();
    let successful_targets = result.get_successful_targets().unwrap_or(&[]);
    for target in configured_targets {
        if contains_target(configured_targets_to_skip, target) {
            skipped.push(target.clone());
            essential_budget -= 1;
            if essential_budget < 0 {
                return essential_budget;
            }
        } else if contains_target(successful_targets, target)
            && !validation_failures.contains(&target.get_lookup_key())
        {
            succeeded.push(target.clone());
            let artifacts_to_print =
                get_artifacts_to_print(&target.get_output_groups(), None, context);
            let empty = artifacts_to_print.is_empty();
            artifacts_to_print_per_target.push(artifacts_to_print);
            if !empty {
                essential_budget -= 1;
                if essential_budget < 0 {
                    return essential_budget;
                }
            }
        } else {
            failed.push(target.clone());
            essential_budget -= 1;
            if essential_budget < 0 {
                return essential_budget;
            }
        }
    }
    essential_budget
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:224 BuildResultPrinter.getArtifactsToPrint
fn get_artifacts_to_print(
    output_groups: &IndexMap<String, NestedSet<Artifact>>,
    files_to_build: Option<&NestedSet<Artifact>>,
    context: &TopLevelArtifactContext,
) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    // For up-to-date targets report generated artifacts, but only if they have associated action
    // and not runfiles trees.
    for artifact in get_important_artifacts(output_groups, files_to_build, context)
        .to_list()
        .iter()
    {
        if should_display(artifact) {
            artifacts.push(artifact.clone());
        }
    }
    artifacts
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:240 BuildResultPrinter.splitAspectsByResultReturnRemaining
fn split_aspects_by_result_return_remaining(
    aspects_to_print: &IndexSet<ActionLookupKeyRef>,
    aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
    context: &TopLevelArtifactContext,
    successful_aspects: Option<&IndexSet<ActionLookupKeyRef>>,
    succeeded: &mut Vec<ActionLookupKeyRef>,
    artifacts_to_print_per_aspect: &mut Vec<Vec<Artifact>>,
    failed: &mut Vec<ActionLookupKeyRef>,
    mut essential_budget: i32,
) -> i32 {
    for aspect in aspects_to_print {
        if successful_aspects.is_some_and(|s| s.contains(aspect)) {
            succeeded.push(aspect.clone());
            let artifacts_to_print = get_artifacts_to_print(
                &aspects.get(aspect).unwrap().get_output_groups(),
                None,
                context,
            );
            let empty = artifacts_to_print.is_empty();
            artifacts_to_print_per_aspect.push(artifacts_to_print);
            if !empty {
                essential_budget -= 1;
                if essential_budget < 0 {
                    return essential_budget;
                }
            }
        } else {
            failed.push(aspect.clone());
            essential_budget -= 1;
            if essential_budget < 0 {
                return essential_budget;
            }
        }
    }
    essential_budget
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:269 BuildResultPrinter.outputConfiguredTargets
fn output_configured_targets(
    out_err: &OutErr,
    pretty_printer: &PathPrettyPrinter,
    succeeded: &[ConfiguredTargetRef],
    artifacts_to_print_per_target: &[Vec<Artifact>],
    failed: &[ConfiguredTargetRef],
    skipped: &[ConfiguredTargetRef],
    omit_nothing_to_build: bool,
) {
    for target in skipped {
        out_err.print_err(&format!(
            "Target {} was skipped\n",
            target.get_original_label()
        ));
    }
    for (i, target) in succeeded.iter().enumerate() {
        let label = target.get_label();
        let artifacts = &artifacts_to_print_per_target[i];
        if artifacts.is_empty() {
            if !omit_nothing_to_build {
                out_err.print_err(&format!("Target {} up-to-date (nothing to build)\n", label));
            }
            continue;
        }
        out_err.print_err(&format!("Target {} up-to-date:\n", label));
        for artifact in artifacts {
            out_err.print_err_ln(&format_artifact_for_show_results(pretty_printer, artifact));
        }
    }
    for target in failed {
        out_err.print_err(&format!("Target {} failed to build\n", target.get_label()));

        // For failed compilation, it is still useful to examine temp artifacts, (ie, preprocessed
        // and assembler files).
        // PORT: `OutputGroupInfo.get(target)` is null when the target has no output groups;
        // `getOutputGroup` of a missing group is the empty set.
        let top_level_provider = target.get_output_groups();
        if let Some(temp_files) = top_level_provider.get(TEMP_FILES) {
            for temp in temp_files.to_list().iter() {
                if temp.get_path().exists() {
                    out_err.print_err_ln(&format!(
                        "  See temp at {}",
                        pretty_printer.get_pretty_path(temp.get_path().as_fragment())
                    ));
                }
            }
        }
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:312 BuildResultPrinter.outputAspects
fn output_aspects(
    out_err: &OutErr,
    pretty_printer: &PathPrettyPrinter,
    succeeded: &[ActionLookupKeyRef],
    aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
    artifacts_to_print_per_aspect: &[Vec<Artifact>],
    failed: &[ActionLookupKeyRef],
    omit_nothing_to_build: bool,
) {
    for (i, aspect) in succeeded.iter().enumerate() {
        let view = aspects.get(aspect).unwrap();
        let label = view.get_label();
        let aspect_name = view.get_aspect_class_name();
        let artifacts = &artifacts_to_print_per_aspect[i];
        if artifacts.is_empty() {
            if !omit_nothing_to_build {
                out_err.print_err(&format!(
                    "Aspect {} of {} up-to-date (nothing to build)\n",
                    aspect_name, label
                ));
            }
            continue;
        }
        out_err.print_err(&format!(
            "Aspect {} of {} up-to-date:\n",
            aspect_name, label
        ));
        for artifact in artifacts {
            out_err.print_err_ln(&format_artifact_for_show_results(pretty_printer, artifact));
        }
    }
    for aspect in failed {
        let view = aspects.get(aspect).unwrap();
        let label = view.get_label();
        let aspect_name = view.get_aspect_class_name();
        out_err.print_err(&format!(
            "Aspect {} of {} failed to build\n",
            aspect_name, label
        ));
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:343 BuildResultPrinter.formatArtifactForShowResults
fn format_artifact_for_show_results(
    pretty_printer: &PathPrettyPrinter,
    artifact: &Artifact,
) -> String {
    format!(
        "  {}",
        pretty_printer.get_pretty_path(artifact.get_path().as_fragment())
    )
}

/// Returns a list of configured targets that should participate in printing.
///
/// Hidden rules and other inserted targets are ignored.
// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:353 BuildResultPrinter.filterTargetsToPrint
fn filter_targets_to_print(
    configured_targets: &[ConfiguredTargetRef],
    generating_rule_lookup: GeneratingRuleLookup<'_>,
) -> Vec<ConfiguredTargetRef> {
    let mut result = Vec::new();
    for configured_target in configured_targets {
        if !should_consider_for_display(configured_target) {
            continue;
        }
        // configuredTarget instanceof OutputFileConfiguredTarget
        if configured_target.get_rule_class_or_kind() == "generated file" {
            // Suppress display of generated files (because they appear underneath
            // their generating rule), EXCEPT those ones which are not part of the
            // filesToBuild of their generating rule (e.g. .par, _deploy.jar
            // files), OR when a user explicitly requests an output file but not
            // its rule.
            let generating_rule = generating_rule_lookup(configured_target);
            let generating_rule_files = generating_rule.generating_rule_files_to_build.to_set();
            if generating_rule
                .files_to_build
                .to_list()
                .iter()
                .all(|artifact| generating_rule_files.contains(artifact))
                && configured_targets
                    .iter()
                    .any(|t| t.get_lookup_key() == generating_rule.generating_rule_key)
            {
                continue;
            }
        }

        result.push(configured_target.clone());
    }
    result
}

/// Splits aspects based on whether they are validation aspects.
// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:385 BuildResultPrinter.partitionAspectKeys
fn partition_aspect_keys(
    use_validation_aspects: bool,
    keys: IndexSet<ActionLookupKeyRef>,
    aspects: &IndexMap<ActionLookupKeyRef, Arc<dyn ConfiguredAspectView>>,
) -> PartitionedAspectKeys {
    if !use_validation_aspects {
        return PartitionedAspectKeys {
            aspects_to_print: keys,
            validation_aspects: Vec::new(),
        };
    }

    let mut aspects_to_print_builder = IndexSet::new();
    let mut validation_aspects_builder = Vec::new();
    for key in keys {
        if aspects.get(&key).unwrap().get_aspect_class_name() == VALIDATION_ASPECT_NAME {
            validation_aspects_builder.push(key);
        } else {
            aspects_to_print_builder.insert(key);
        }
    }
    PartitionedAspectKeys {
        aspects_to_print: aspects_to_print_builder,
        validation_aspects: validation_aspects_builder,
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:404 BuildResultPrinter.PartitionedAspectKeys
struct PartitionedAspectKeys {
    aspects_to_print: IndexSet<ActionLookupKeyRef>,
    validation_aspects: Vec<ActionLookupKeyRef>,
}
