//! Ports of `analysis/test/InstrumentedFilesInfo.java`,
//! `analysis/test/InstrumentedFilesCollector.java` (with `InstrumentationSpec`) and
//! `analysis/test/CoverageCommon.java` (`coverage_common`).

use std::fmt;
use std::sync::LazyLock;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::starlark_defs::builtin_providers::INSTRUMENTED_FILES_INFO;
use blaze_packages::types::{FileType, FileTypeSet, LabelClass};
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneOr;
use starlark::values::tuple::TupleRef;
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, UnpackValue, Value, starlark_value,
};
use starlark_map::StarlarkHasher;

use crate::configured_target::ConfiguredTarget;
use crate::info::{InfoProviderKey, downcast_info, freeze_native_info};
use crate::providers::file_provider::FileProvider;
use crate::providers::run_environment_info::{dict_cast_string_string, sequence_cast_strings};
use crate::rule::builtin_restriction;
use crate::rule::config_util;
use crate::rule::depsets::{artifacts_to_depset, depset_to_artifacts, depset_value};
use crate::rule::rule_context::RuleContext;

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// An element of `reported_to_actual_sources` (Java: a Starlark `Tuple`).
///
/// PORT: Java keeps the Starlark tuples; the only reader is `LazyWriteNestedSetOfTupleAction`
/// (`TestActionBuilder`), whose `Tuple` keeps what it reads of each (`String.valueOf` of the
/// elements and the repr).
pub type ReportedToActualSource =
    blaze_actions::analysis::actions::lazy_write_nested_set_of_tuple_action::Tuple;

/// An implementation class for the InstrumentedFilesProvider interface.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:31 InstrumentedFilesInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct InstrumentedFilesInfo {
    #[allocative(skip)]
    instrumented_files: NestedSet<Artifact>,
    #[allocative(skip)]
    instrumentation_metadata_files: NestedSet<Artifact>,
    #[allocative(skip)]
    baseline_coverage_artifacts: NestedSet<Artifact>,
    #[allocative(skip)]
    coverage_support_files: NestedSet<Artifact>,
    #[allocative(skip)]
    coverage_environment: IndexMap<String, String>,
    #[allocative(skip)]
    reported_to_actual_sources: NestedSet<ReportedToActualSource>,
}

starlark_simple_value!(InstrumentedFilesInfo);

fn empty_artifacts() -> NestedSet<Artifact> {
    NestedSetBuilder::empty_set(Order::STABLE_ORDER)
}

// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:36 InstrumentedFilesInfo.EMPTY
static EMPTY: LazyLock<OwnedFrozenValue> = LazyLock::new(|| {
    freeze_native_info(InstrumentedFilesInfo {
        instrumented_files: empty_artifacts(),
        instrumentation_metadata_files: empty_artifacts(),
        baseline_coverage_artifacts: empty_artifacts(),
        coverage_support_files: empty_artifacts(),
        coverage_environment: IndexMap::new(),
        reported_to_actual_sources: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
    })
});

/// Java `InstrumentedFilesInfo.EMPTY` as a frozen info.
pub fn empty_instrumented_files_info() -> OwnedFrozenValue {
    EMPTY.clone()
}

impl InstrumentedFilesInfo {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:52 InstrumentedFilesInfo.<init>
    pub fn new(
        instrumented_files: NestedSet<Artifact>,
        instrumentation_metadata_files: NestedSet<Artifact>,
        baseline_coverage_artifacts: NestedSet<Artifact>,
        coverage_support_files: NestedSet<Artifact>,
        coverage_environment: IndexMap<String, String>,
        reported_to_actual_sources: NestedSet<ReportedToActualSource>,
    ) -> InstrumentedFilesInfo {
        InstrumentedFilesInfo {
            instrumented_files,
            instrumentation_metadata_files,
            baseline_coverage_artifacts,
            coverage_support_files,
            coverage_environment,
            reported_to_actual_sources,
        }
    }

    /// The transitive closure of instrumented source files.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:73 InstrumentedFilesInfo.getInstrumentedFiles
    pub fn get_instrumented_files(&self) -> &NestedSet<Artifact> {
        &self.instrumented_files
    }

    /// Returns a collection of instrumentation metadata files.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:83 InstrumentedFilesInfo.getInstrumentationMetadataFiles
    pub fn get_instrumentation_metadata_files(&self) -> &NestedSet<Artifact> {
        &self.instrumentation_metadata_files
    }

    /// The transitive closure of instrumented source files for baseline coverage.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:96 InstrumentedFilesInfo.getBaselineCoverageArtifacts
    pub fn get_baseline_coverage_artifacts(&self) -> &NestedSet<Artifact> {
        &self.baseline_coverage_artifacts
    }

    /// Extra files that are needed on the inputs of test actions for coverage collection to
    /// happen, for example, `gcov`.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:104 InstrumentedFilesInfo.getCoverageSupportFiles
    pub fn get_coverage_support_files(&self) -> &NestedSet<Artifact> {
        &self.coverage_support_files
    }

    /// Environment variables that need to be set for tests collecting code coverage.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:116 InstrumentedFilesInfo.getCoverageEnvironment
    pub fn get_coverage_environment(&self) -> &IndexMap<String, String> {
        &self.coverage_environment
    }

    /// A map of source files reported in coverage to their actual source files.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:131 InstrumentedFilesInfo.getReportedToActualSources
    pub fn get_reported_to_actual_sources(&self) -> &NestedSet<ReportedToActualSource> {
        &self.reported_to_actual_sources
    }
}

impl fmt::Display for InstrumentedFilesInfo {
    /// `NativeInfo.repr`: `struct(` and the legacy fields (`Starlark.dir`, natural order, the
    /// builtin functions `get_coverage_environment` and `get_coverage_support_files` skipped)
    /// with their values' `repr`.
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "struct(instrumented_files = {}, metadata_files = {})",
            artifacts_to_depset(&self.instrumented_files)
                .value()
                .to_repr(),
            artifacts_to_depset(&self.instrumentation_metadata_files)
                .value()
                .to_repr()
        )
    }
}

/// The struct fields (`structField = true`); `get_coverage_environment` and
/// `get_coverage_support_files` are methods ([`instrumented_files_info_methods`]).
const FIELDS: [&str; 2] = ["instrumented_files", "metadata_files"];

fn this_instrumented_files_info<'v>(this: Value<'v>) -> &'v InstrumentedFilesInfo {
    starlark::values::ValueLike::downcast_ref::<InstrumentedFilesInfo>(this)
        .expect("InstrumentedFilesInfo method on another value")
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/InstrumentedFilesInfoApi.java:31 InstrumentedFilesInfoApi
#[starlark_module]
fn instrumented_files_info_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:122 InstrumentedFilesInfo.getCoverageEnvironmentForStarlark
    fn get_coverage_environment<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        builtin_restriction::fail_if_called_outside_default_allowlist(eval)?;
        // The ImmutableMap becomes an immutable Dict (Starlark.fromJava).
        let heap = eval.heap();
        let dict = heap.alloc(starlark::values::dict::AllocDict(
            this_instrumented_files_info(this)
                .coverage_environment
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        ));
        blaze_starlark::dict::dict_immutable_copy_of(heap, dict)
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:111 InstrumentedFilesInfo.getCoverageSupportFilesForStarlark
    fn get_coverage_support_files<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        builtin_restriction::fail_if_called_outside_default_allowlist(eval)?;
        Ok(depset_value(&artifacts_to_depset(
            &this_instrumented_files_info(this).coverage_support_files,
        )))
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/InstrumentedFilesInfoApi.java:31 InstrumentedFilesInfoApi
#[starlark_value(type = "InstrumentedFilesInfo")]
impl<'v> StarlarkValue<'v> for InstrumentedFilesInfo {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:78 InstrumentedFilesInfo.getInstrumentedFilesForStarlark
            "instrumented_files" => {
                Some(depset_value(&artifacts_to_depset(&self.instrumented_files)))
            }
            // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:88 InstrumentedFilesInfo.getInstrumentationMetadataFilesForStarlark
            "metadata_files" => Some(depset_value(&artifacts_to_depset(
                &self.instrumentation_metadata_files,
            ))),
            _ => None,
        }
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("InstrumentedFilesInfo", instrumented_files_info_methods);
        Some(RES.methods())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut v: Vec<String> = FIELDS.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(INSTRUMENTED_FILES_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// The set of file types and attributes to visit to collect instrumented files for a certain
/// rule type.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:178 InstrumentedFilesCollector.InstrumentationSpec
#[derive(Clone, Debug)]
pub struct InstrumentationSpec {
    instrumented_file_types: FileTypeSet,
    /// The list of attributes which should be checked for sources.
    source_attributes: Vec<String>,
    /// The list of attributes from which to collect transitive coverage information.
    dependency_attributes: Vec<String>,
}

impl InstrumentationSpec {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:200 InstrumentationSpec.<init>
    pub fn new(instrumented_file_types: FileTypeSet) -> InstrumentationSpec {
        InstrumentationSpec {
            instrumented_file_types,
            source_attributes: Vec::new(),
            dependency_attributes: Vec::new(),
        }
    }

    /// Returns a new instance with the same instrumented file types and dependency attributes,
    /// but with the given source attributes.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:205 InstrumentationSpec.withSourceAttributes
    pub fn with_source_attributes<S: AsRef<str>>(self, attributes: &[S]) -> InstrumentationSpec {
        InstrumentationSpec {
            source_attributes: attributes.iter().map(|s| s.as_ref().to_owned()).collect(),
            ..self
        }
    }

    /// Returns a new instance with the same instrumented file types and source attributes, but
    /// with the given dependency attributes.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:222 InstrumentationSpec.withDependencyAttributes
    pub fn with_dependency_attributes<S: AsRef<str>>(
        self,
        attributes: &[S],
    ) -> InstrumentationSpec {
        InstrumentationSpec {
            dependency_attributes: attributes.iter().map(|s| s.as_ref().to_owned()).collect(),
            ..self
        }
    }
}

/// The builder of `InstrumentedFilesInfo` (Java `InstrumentedFilesCollector.InstrumentedFilesInfoBuilder`).
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:238 InstrumentedFilesCollector.InstrumentedFilesInfoBuilder
struct InstrumentedFilesInfoBuilder<'a> {
    rule_context: &'a RuleContext,
    instrumented_files_builder: NestedSetBuilder<Artifact>,
    metadata_files_builder: NestedSetBuilder<Artifact>,
    baseline_coverage_artifacts_builder: NestedSetBuilder<Artifact>,
    coverage_support_files_builder: NestedSetBuilder<Artifact>,
    /// Java `ImmutableMap.Builder` built with `buildKeepingLast`.
    coverage_environment_builder: IndexMap<String, String>,
    reported_to_actual_sources: NestedSet<ReportedToActualSource>,
    local_sources: Option<NestedSet<Artifact>>,
    local_baseline_coverage_artifacts: Option<Vec<Artifact>>,
    /// `ruleContext.registerAction`. PORT: inside a Starlark rule implementation
    /// (`coverage_common.instrumented_files_info`) the action joins the thread's ordered action
    /// list (`thread_payload::register_action`), so it keeps its place among the actions the
    /// implementation registered before and after it.
    registrar: Option<&'a dyn Fn(blaze_actions::ActionRef)>,
}

impl<'a> InstrumentedFilesInfoBuilder<'a> {
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:250 InstrumentedFilesInfoBuilder.<init>
    fn new(
        rule_context: &'a RuleContext,
        coverage_support_files: &NestedSet<Artifact>,
        reported_to_actual_sources: NestedSet<ReportedToActualSource>,
    ) -> Self {
        let mut coverage_support_files_builder = NestedSetBuilder::stable_order();
        coverage_support_files_builder.add_transitive(coverage_support_files);
        InstrumentedFilesInfoBuilder {
            rule_context,
            instrumented_files_builder: NestedSetBuilder::stable_order(),
            metadata_files_builder: NestedSetBuilder::stable_order(),
            baseline_coverage_artifacts_builder: NestedSetBuilder::stable_order(),
            coverage_support_files_builder,
            coverage_environment_builder: IndexMap::new(),
            reported_to_actual_sources,
            local_sources: None,
            local_baseline_coverage_artifacts: None,
            registrar: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:272 InstrumentedFilesInfoBuilder.addFromDependency
    fn add_from_dependency(&mut self, dep: &ConfiguredTarget) {
        let Some(info) = crate::configuredtargets::get_info(dep, &INSTRUMENTED_FILES_INFO.key())
        else {
            return;
        };
        let provider =
            downcast_info::<InstrumentedFilesInfo>(&info).expect("InstrumentedFilesInfo");
        self.instrumented_files_builder
            .add_transitive(provider.get_instrumented_files());
        self.metadata_files_builder
            .add_transitive(provider.get_instrumentation_metadata_files());
        self.baseline_coverage_artifacts_builder
            .add_transitive(provider.get_baseline_coverage_artifacts());
        self.coverage_support_files_builder
            .add_transitive(provider.get_coverage_support_files());
        for (k, v) in provider.get_coverage_environment() {
            // ImmutableMap.Builder.buildKeepingLast: a repeated key keeps the position of its
            // first occurrence with its last value (IndexMap.insert).
            self.coverage_environment_builder
                .insert(k.clone(), v.clone());
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:283 InstrumentedFilesInfoBuilder.setLocalSources
    fn set_local_sources(&mut self, local_sources: Vec<Artifact>) {
        let mut b = NestedSetBuilder::stable_order();
        b.add_all(local_sources);
        let local_sources_nested_set = b.build();
        self.instrumented_files_builder
            .add_transitive(&local_sources_nested_set);
        self.local_sources = Some(local_sources_nested_set);
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:290 InstrumentedFilesInfoBuilder.setBaselineCoverageFiles
    fn set_baseline_coverage_files(&mut self, files: Vec<Artifact>) {
        self.local_baseline_coverage_artifacts = Some(files);
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:294 InstrumentedFilesInfoBuilder.addMetadataFiles
    fn add_metadata_files(&mut self, files: Vec<Artifact>) {
        self.metadata_files_builder.add_all(files);
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:298 InstrumentedFilesInfoBuilder.build
    fn build(mut self) -> InstrumentedFilesInfo {
        if let Some(local_sources) = &self.local_sources {
            if !local_sources.is_empty() {
                match self.local_baseline_coverage_artifacts.take() {
                    Some(files) => {
                        self.baseline_coverage_artifacts_builder.add_all(files);
                    }
                    None => {
                        let baseline_coverage_action =
                            crate::test::baseline_coverage_action::BaselineCoverageAction::create(
                                self.rule_context,
                                local_sources.clone(),
                            );
                        let primary_output =
                            blaze_actions::Action::get_primary_output(&baseline_coverage_action);
                        let action: blaze_actions::ActionRef =
                            std::sync::Arc::new(baseline_coverage_action);
                        match self.registrar {
                            Some(register) => register(action),
                            None => self.rule_context.register_action(action),
                        }
                        self.baseline_coverage_artifacts_builder.add(primary_output);
                    }
                }
            }
        }
        InstrumentedFilesInfo::new(
            self.instrumented_files_builder.build(),
            self.metadata_files_builder.build(),
            self.baseline_coverage_artifacts_builder.build(),
            self.coverage_support_files_builder.build(),
            self.coverage_environment_builder,
            self.reported_to_actual_sources,
        )
    }
}

/// A helper class for collecting instrumented files and metadata for a target.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:50 InstrumentedFilesCollector
pub struct InstrumentedFilesCollector;

impl InstrumentedFilesCollector {
    /// Forwards any instrumented files from the given target's dependencies (as defined in
    /// `dependency_attributes`) for further export. No files from this target are considered
    /// instrumented.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:62 InstrumentedFilesCollector.forward
    pub fn forward(rule_context: &RuleContext, dependency_attributes: &[&str]) -> OwnedFrozenValue {
        Self::collect(
            rule_context,
            &InstrumentationSpec::new(FileTypeSet::NoFile)
                .with_dependency_attributes(dependency_attributes),
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        )
    }

    /// Forwards the instrumented files of all non-tool dependencies.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:72 InstrumentedFilesCollector.forwardAll
    pub fn forward_all(rule_context: &RuleContext) -> OwnedFrozenValue {
        if !config_util::is_code_coverage_enabled(rule_context.get_configuration()) {
            return empty_instrumented_files_info();
        }
        let mut builder = InstrumentedFilesInfoBuilder::new(
            rule_context,
            &empty_artifacts(),
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        );
        for dep in Self::get_all_non_tool_prerequisites(rule_context) {
            builder.add_from_dependency(&dep);
        }
        freeze_native_info(builder.build())
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:84 InstrumentedFilesCollector.collect
    pub fn collect(
        rule_context: &RuleContext,
        spec: &InstrumentationSpec,
        reported_to_actual_sources: NestedSet<ReportedToActualSource>,
    ) -> OwnedFrozenValue {
        Self::collect_full(
            rule_context,
            spec,
            &empty_artifacts(),
            &IndexMap::new(),
            reported_to_actual_sources,
            None,
            None,
        )
    }

    /// Collects transitive instrumentation data from dependencies, collects local source files
    /// from dependencies, collects local metadata files by traversing the action graph of the
    /// current configured target, collect rule-specific instrumentation support file sources and
    /// environment variables, and creates an `InstrumentedFilesInfo` from it.
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:108 InstrumentedFilesCollector.collect
    pub fn collect_full(
        rule_context: &RuleContext,
        spec: &InstrumentationSpec,
        coverage_support_files: &NestedSet<Artifact>,
        coverage_environment: &IndexMap<String, String>,
        reported_to_actual_sources: NestedSet<ReportedToActualSource>,
        additional_metadata: Option<Vec<Artifact>>,
        baseline_coverage_files: Option<Vec<Artifact>>,
    ) -> OwnedFrozenValue {
        Self::collect_full_with_registrar(
            rule_context,
            spec,
            coverage_support_files,
            coverage_environment,
            reported_to_actual_sources,
            additional_metadata,
            baseline_coverage_files,
            None,
        )
    }

    /// [`Self::collect_full`] with the `registerAction` of a Starlark rule implementation (see
    /// `InstrumentedFilesInfoBuilder::registrar`).
    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:108 InstrumentedFilesCollector.collect
    #[allow(clippy::too_many_arguments)]
    pub fn collect_full_with_registrar(
        rule_context: &RuleContext,
        spec: &InstrumentationSpec,
        coverage_support_files: &NestedSet<Artifact>,
        coverage_environment: &IndexMap<String, String>,
        reported_to_actual_sources: NestedSet<ReportedToActualSource>,
        additional_metadata: Option<Vec<Artifact>>,
        baseline_coverage_files: Option<Vec<Artifact>>,
        registrar: Option<&dyn Fn(blaze_actions::ActionRef)>,
    ) -> OwnedFrozenValue {
        let configuration = rule_context.get_configuration();
        if !config_util::is_code_coverage_enabled(configuration) {
            return empty_instrumented_files_info();
        }

        let mut builder = InstrumentedFilesInfoBuilder::new(
            rule_context,
            coverage_support_files,
            reported_to_actual_sources,
        );
        builder.registrar = registrar;

        for dep in Self::get_prerequisites_for_attributes(rule_context, &spec.dependency_attributes)
        {
            builder.add_from_dependency(&dep);
        }

        for (k, v) in coverage_environment {
            // ImmutableMap.Builder.buildKeepingLast: a repeated key keeps the position of its
            // first occurrence with its last value (IndexMap.insert).
            builder
                .coverage_environment_builder
                .insert(k.clone(), v.clone());
        }

        // Add the local sources.
        let mut local_sources: Vec<Artifact> = Vec::new();
        if Self::should_include_local_sources(
            configuration,
            rule_context.get_label(),
            rule_context.is_test_target(),
        ) {
            for dep in Self::get_prerequisites_for_attributes(rule_context, &spec.source_attributes)
            {
                if let Some(fp) = crate::configuredtargets::get_provider::<FileProvider>(&dep) {
                    for artifact in fp.get_files_to_build().to_list().iter() {
                        if Self::should_include_artifact(configuration, artifact)
                            && spec.instrumented_file_types.apply(artifact.get_filename())
                            && !local_sources.contains(artifact)
                        {
                            local_sources.push(artifact.clone());
                        }
                    }
                }
            }
        }
        builder.set_local_sources(local_sources);
        if let Some(files) = baseline_coverage_files {
            builder.set_baseline_coverage_files(files);
        }
        if let Some(files) = additional_metadata {
            builder.add_metadata_files(files);
        }
        freeze_native_info(builder.build())
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:162 InstrumentedFilesCollector.shouldIncludeLocalSources
    pub fn should_include_local_sources(
        config: &blaze_build_config::build_configuration_value::BuildConfigurationValue,
        label: &blaze_cmdline::Label,
        is_test: bool,
    ) -> bool {
        (config_util::should_instrument_test_targets(config) || !is_test)
            && config_util::instrumentation_filter_is_included(config, &label.to_string())
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:168 InstrumentedFilesCollector.shouldIncludeArtifact
    pub fn should_include_artifact(
        config: &blaze_build_config::build_configuration_value::BuildConfigurationValue,
        artifact: &Artifact,
    ) -> bool {
        artifact.is_source_artifact()
            || config_util::should_collect_code_coverage_for_generated_files(config)
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:324 InstrumentedFilesCollector.getPrerequisitesForAttributes
    fn get_prerequisites_for_attributes(
        rule_context: &RuleContext,
        attribute_names: &[String],
    ) -> Vec<ConfiguredTarget> {
        let mut prerequisites = Vec::new();
        for attribute_name in attribute_names {
            if let Some(attribute) = rule_context
                .get_rule()
                .get_rule_class_object()
                .get_attribute_provider()
                .get_attribute_by_name_maybe(attribute_name)
            {
                if attribute.get_type().get_label_class() == LabelClass::Dependency {
                    prerequisites.extend(rule_context.get_prerequisites(attribute.get_name()));
                }
            }
        }
        prerequisites
    }

    // Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:340 InstrumentedFilesCollector.getAllNonToolPrerequisites
    fn get_all_non_tool_prerequisites(rule_context: &RuleContext) -> Vec<ConfiguredTarget> {
        use blaze_packages::target::RuleOrMacroInstance;
        let mut prerequisites = Vec::new();
        for attribute in rule_context.get_rule().get_attributes() {
            if !attribute.is_tool_dependency()
                && attribute.get_type().get_label_class() == LabelClass::Dependency
            {
                prerequisites.extend(rule_context.get_prerequisites(attribute.get_name()));
            }
        }
        prerequisites
    }
}

// -------------------------------------------------------------------------------------------
// coverage_common

/// The `coverage_common` module (Java `CoverageCommon`).
// Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:38 CoverageCommon
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct CoverageCommon;

starlark_simple_value!(CoverageCommon);

impl fmt::Display for CoverageCommon {
    // Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:154 CoverageCommon.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<coverage_common>")
    }
}

#[starlark_value(type = "coverage_common")]
impl<'v> StarlarkValue<'v> for CoverageCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("coverage_common", coverage_common_methods);
        Some(RES.methods())
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'coverage_common'".to_owned()))
    }
}

/// `Depset.cast(x, Tuple.class, "reported_to_actual_sources")` as Rust pairs.
fn reported_to_actual_sources_of(v: Value) -> starlark::Result<NestedSet<ReportedToActualSource>> {
    let what = "reported_to_actual_sources";
    let Some(d) = Depset::from_value(v) else {
        return Err(errorf(format!(
            "for {what}, got {}, want a depset of tuple",
            v.get_type()
        )));
    };
    if !d.is_empty() && d.element_type() != "tuple" {
        return Err(errorf(format!(
            "for '{what}', got a depset of '{}', expected a depset of 'tuple'",
            d.element_type()
        )));
    }
    // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
    // (the depset's one NestedSet, see rule::depsets).
    crate::rule::depsets::depset_to_nested_set_shared(v, d, &mut |x: Value| {
        let t = TupleRef::from_value(x).ok_or_else(|| {
            errorf(format!(
                "for '{what}', got a depset of '{}', expected a depset of 'tuple'",
                x.get_type()
            ))
        })?;
        // String.valueOf(element) of each element (Starlark `str`), and Starlark.repr(tuple).
        Ok(ReportedToActualSource::new(
            t.content().iter().map(|v| v.to_str()).collect(),
            x.to_repr(),
        ))
    })
}

#[starlark_module]
fn coverage_common_methods(builder: &mut MethodsBuilder) {
    /// Creates a new `InstrumentedFilesInfo` instance. Use this provider to communicate
    /// coverage-related attributes of the current build rule.
    // Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:47 CoverageCommon.instrumentedFilesInfo
    fn instrumented_files_info<'v>(
        this: Value<'v>,
        ctx: Value<'v>,
        #[starlark(require = named, default = UnpackListOrTuple::default())]
        source_attributes: UnpackListOrTuple<Value<'v>>,
        #[starlark(require = named, default = UnpackListOrTuple::default())] dependency_attributes: UnpackListOrTuple<Value<'v>>,
        #[starlark(require = named)] coverage_support_files: Option<Value<'v>>,
        #[starlark(require = named)] coverage_environment: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] extensions: NoneOr<Value<'v>>,
        #[starlark(require = named, default = UnpackListOrTuple::default())]
        metadata_files: UnpackListOrTuple<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] reported_to_actual_sources: NoneOr<
            Value<'v>,
        >,
        #[starlark(require = named, default = NoneOr::None)] baseline_coverage_files: NoneOr<
            Value<'v>,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let extensions_list: Option<Vec<String>> = match extensions {
            NoneOr::None => None,
            NoneOr::Other(v) => Some(sequence_cast_strings(v, "extensions")?),
        };
        let reported: NestedSet<ReportedToActualSource> = match reported_to_actual_sources {
            NoneOr::None => NestedSetBuilder::create(Order::STABLE_ORDER, std::iter::empty()),
            NoneOr::Other(v) => reported_to_actual_sources_of(v)?,
        };
        let baseline_coverage_files_list: Option<Vec<Artifact>> = match baseline_coverage_files {
            NoneOr::None => None,
            NoneOr::Other(v) => Some(sequence_cast_artifacts(v, "baseline_coverage_files")?),
        };
        let environment_dict = match coverage_environment {
            None => IndexMap::new(),
            Some(v) => dict_cast_string_string(v, "coverage_environment")?,
        };
        let mut support_files_builder: NestedSetBuilder<Artifact> =
            NestedSetBuilder::stable_order();
        match coverage_support_files {
            None => {}
            Some(support_files) => {
                if Depset::from_value(support_files).is_some() {
                    support_files_builder.add_transitive(&depset_to_artifacts(
                        support_files,
                        "coverage_support_files",
                    )?);
                } else if let Some(seq) = UnpackListOrTuple::<Value>::unpack_value(support_files)? {
                    for (i, element) in seq.items.iter().enumerate() {
                        if Depset::from_value(*element).is_some() {
                            support_files_builder.add_transitive(&depset_to_artifacts(
                                *element,
                                "coverage_support_files",
                            )?);
                        } else if let Some(artifact) = artifact_from_value(*element) {
                            support_files_builder.add(artifact);
                        } else {
                            return Err(errorf(format!(
                                "at index {i} of coverage_support_files, got element of type {}, want one of depset, File or FilesToRunProvider",
                                element.get_type()
                            )));
                        }
                    }
                } else {
                    panic!("IllegalStateException");
                }
            }
        }
        if !support_files_builder.is_empty() || !reported.is_empty() || !environment_dict.is_empty()
        {
            builtin_restriction::fail_if_called_outside_default_allowlist(eval)?;
        }
        let source_attributes =
            sequence_cast_strings_items(&source_attributes.items, "source_attributes")?;
        let dependency_attributes =
            sequence_cast_strings_items(&dependency_attributes.items, "dependency_attributes")?;
        let metadata = sequence_cast_artifact_items(&metadata_files.items, "metadata_files")?;
        // Java: the `ctx` parameter is a StarlarkRuleContext; `getRuleContext()`.
        let starlark_rule_context =
            crate::starlark::starlark_rule_context::starlark_rule_context_from_value(ctx)
                .ok_or_else(|| {
                    errorf(format!(
                        "in call to instrumented_files_info(), parameter 'ctx' got value of type '{}', want 'ctx'",
                        ctx.get_type()
                    ))
                })?;
        let handle = starlark_rule_context.get_rule_context();
        let rule_context = handle.borrow();
        let eval_ref: &Evaluator = eval;
        let info = create_instrumented_files_info(
            &rule_context,
            &source_attributes,
            &dependency_attributes,
            support_files_builder.build(),
            environment_dict,
            extensions_list,
            Some(metadata),
            reported,
            baseline_coverage_files_list,
            &|action| {
                // ruleContext.registerAction from inside the rule implementation.
                if let Err(e) = crate::starlark::thread_payload::register_action(eval_ref, action) {
                    panic!("IllegalStateException: {e}");
                }
            },
        );
        Ok(eval.heap().access_owned_frozen_value(&info))
    }
}

/// Java `Sequence.cast(x, String.class, what)` over already-unpacked items.
fn sequence_cast_strings_items(items: &[Value], what: &str) -> starlark::Result<Vec<String>> {
    items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v.unpack_str().map(str::to_owned).ok_or_else(|| {
                errorf(format!(
                    "at index {i} of {what}, got element of type {}, want string",
                    v.get_type()
                ))
            })
        })
        .collect()
}

/// Java `Sequence.cast(x, Artifact.class, what)` over already-unpacked items.
fn sequence_cast_artifact_items(items: &[Value], what: &str) -> starlark::Result<Vec<Artifact>> {
    items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            artifact_from_value(*v).ok_or_else(|| {
                errorf(format!(
                    "at index {i} of {what}, got element of type {}, want File",
                    v.get_type()
                ))
            })
        })
        .collect()
}

/// Java `Sequence.cast(x, Artifact.class, what)`.
fn sequence_cast_artifacts(x: Value, what: &str) -> starlark::Result<Vec<Artifact>> {
    let Some(seq) = UnpackListOrTuple::<Value>::unpack_value(x)? else {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    };
    sequence_cast_artifact_items(&seq.items, what)
}

// Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:111 CoverageCommon.createInstrumentedFilesInfo
fn create_instrumented_files_info(
    rule_context: &RuleContext,
    source_attributes: &[String],
    dependency_attributes: &[String],
    support_files: NestedSet<Artifact>,
    environment: IndexMap<String, String>,
    extensions: Option<Vec<String>>,
    metadata_files: Option<Vec<Artifact>>,
    reported_to_actual_sources: NestedSet<ReportedToActualSource>,
    baseline_coverage_files: Option<Vec<Artifact>>,
    registrar: &dyn Fn(blaze_actions::ActionRef),
) -> OwnedFrozenValue {
    let mut file_type_set = FileTypeSet::AnyFile;
    if let Some(extensions) = extensions {
        if extensions.is_empty() {
            file_type_set = FileTypeSet::NoFile;
        } else {
            let file_types: Vec<FileType> = extensions
                .iter()
                .map(|e| FileType::of(&[e.as_str()]))
                .collect();
            file_type_set = FileTypeSet::of(file_types);
        }
    }
    let instrumentation_spec = InstrumentationSpec::new(file_type_set)
        .with_source_attributes(source_attributes)
        .with_dependency_attributes(dependency_attributes);
    InstrumentedFilesCollector::collect_full_with_registrar(
        rule_context,
        &instrumentation_spec,
        /* coverage_support_files= */ &support_files,
        /* coverage_environment= */ &environment,
        /* reported_to_actual_sources= */ reported_to_actual_sources,
        /* additional_metadata= */ metadata_files,
        /* baseline_coverage_files= */ baseline_coverage_files,
        Some(registrar),
    )
}

/// Registers the analysis-capable `coverage_common` (the lead substitutes it for W3's).
// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingBootstrap.java:48 TestingBootstrap.addBindingsToBuilder
pub fn register_coverage_common(builder: &mut GlobalsBuilder) {
    builder.set("coverage_common", CoverageCommon);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_info_has_empty_sets() {
        let e = empty_instrumented_files_info();
        let info = downcast_info::<InstrumentedFilesInfo>(&e).unwrap();
        assert!(info.get_instrumented_files().is_empty());
        assert!(info.get_coverage_environment().is_empty());
    }

    #[test]
    #[ignore = "needs a RuleContext with --collect_code_coverage (InstrumentedFilesCollectorTest)"]
    fn collect_with_coverage() {}
}
