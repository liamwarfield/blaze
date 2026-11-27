//! Port of `rules/starlarkdocextract/StarlarkDocExtract.java`: the implementation of the
//! `starlark_doc_extract` rule.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::actions::binary_file_write_action::{BinaryFileWriteAction, wrap};
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::analysis::runfiles::RunfilesBuilder;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::{FileProvider, RunfilesProvider};
use blaze_analysis::rule::rule_configured_target_builder::RuleConfiguredTargetBuilder;
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::{RuleErrorConsumer, RuleErrorException};
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, RepositoryMapping};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_packages::types::{AttrValue, Type};
use blaze_sky_loading::bzl_load::{BzlLoadFailedException, BzlLoadKey, BzlLoadValue};
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_starlarkdocextract::label_renderer::LabelRenderer;
use blaze_starlarkdocextract::module_info_extractor::{DocumentedModule, ModuleInfoExtractor};

// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:62 StarlarkDocExtract.SRC_ATTR
const SRC_ATTR: &str = "src";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:63 StarlarkDocExtract.DEPS_ATTR
const DEPS_ATTR: &str = "deps";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:64 StarlarkDocExtract.SYMBOL_NAMES_ATTR
const SYMBOL_NAMES_ATTR: &str = "symbol_names";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:65 StarlarkDocExtract.RENDER_MAIN_REPO_NAME
const RENDER_MAIN_REPO_NAME: &str = "render_main_repo_name";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:66 StarlarkDocExtract.ALLOW_UNUSED_DOC_COMMENTS
const ALLOW_UNUSED_DOC_COMMENTS: &str = "allow_unused_doc_comments";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:67 StarlarkDocExtract.BINARYPROTO_OUT
const BINARYPROTO_OUT: &str = "%{name}.binaryproto";
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:68 StarlarkDocExtract.TEXTPROTO_OUT
const TEXTPROTO_OUT: &str = "%{name}.textproto";

/// Implementation of the `starlark_doc_extract` rule.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:61 StarlarkDocExtract
pub struct StarlarkDocExtract;

/// The main repository's `RepositoryMappingValue` as this rule reads it.
struct MainRepositoryMapping {
    repository_mapping: RepositoryMapping,
    associated_module_name: Option<String>,
}

impl RuleConfiguredTargetFactory for StarlarkDocExtract {
    // Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:70 StarlarkDocExtract.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let Some(main_repository_mapping_value) = get_main_repository_mapping_value(rule_context)?
        else {
            return Ok(None);
        };
        let repository_mapping = main_repository_mapping_value.repository_mapping.clone();
        let Some(module) = load_module(rule_context, &repository_mapping)? else {
            // Skyframe restart
            return Ok(None);
        };
        verify_module_deps(rule_context, &module, &repository_mapping)?;
        let mut main_repo_name: Option<String> = None;
        if bool_attr(rule_context, RENDER_MAIN_REPO_NAME) {
            main_repo_name = main_repository_mapping_value.associated_module_name.clone();
            if main_repo_name.is_none() {
                main_repo_name = Some(rule_context.get_workspace_name().to_owned());
            }
        }
        let module_info = get_module_info(
            rule_context,
            &module,
            LabelRenderer::new(repository_mapping, main_repo_name),
        )?;

        let files_to_build = NestedSetBuilder::create(
            Order::STABLE_ORDER,
            [create_binary_proto_output(rule_context, &module_info)],
        );
        // Textproto output isn't in filesToBuild: we want to create it only if explicitly
        // requested.
        create_text_proto_output(rule_context, &module_info);

        let mut runfiles = RunfilesBuilder::new(rule_context.get_workspace_name());
        blaze_analysis::rule::runfiles_util::add_runfiles(
            &mut runfiles,
            rule_context,
            blaze_analysis::providers::file_provider::RunfilesMapping::DefaultRunfiles,
        );
        runfiles.add_transitive_artifacts(&files_to_build);
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .set_files_to_build(files_to_build)
            .add_provider(RunfilesProvider::simple(runfiles.build()));
        Ok(builder.build()?)
    }
}

fn bool_attr(rule_context: &RuleContext, name: &str) -> bool {
    matches!(
        rule_context.attributes().get(name, Type::Boolean),
        Some(AttrValue::Bool(true))
    )
}

/// Loads the Starlark module from the source file given by the rule's `src` attribute; `None` on
/// a Skyframe restart.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:121 StarlarkDocExtract.loadModule
fn load_module(
    rule_context: &RuleContext,
    repository_mapping: &RepositoryMapping,
) -> Result<Option<Arc<BzlLoadValue>>, CreateConfiguredTargetError> {
    // PORT: skipped, profiling ("BzlDocDump.loadModule").
    // Note attr schema validates that src is a .bzl or .scl file.
    let label = get_source_file_label(rule_context, SRC_ATTR, repository_mapping)?;

    // TODO(b/276733504): support loading modules in @_builtins
    let loaded = blaze_analysis::skyframe_env::get_skyframe_env(|env| {
        env.get_value_or_throw::<BzlLoadKey, BzlLoadFailedException>(BzlLoadKey::key_for_build(
            label,
        ))
    });
    match loaded {
        Err(_) => Err(CreateConfiguredTargetError::Interrupted),
        Ok(Err(e)) => {
            rule_context.attribute_error(SRC_ATTR, e.get_message());
            Err(RuleErrorException::new(e.get_message()).into())
        }
        // Skyframe restart
        Ok(Ok(None)) => Ok(None),
        Ok(Ok(Some(value))) => Ok(Some(value)),
    }
}

/// Retrieves the label of the singular source artifact from a given attribute.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:163 StarlarkDocExtract.getSourceFileLabel
fn get_source_file_label(
    rule_context: &RuleContext,
    attr_name: &str,
    repository_mapping: &RepositoryMapping,
) -> Result<Label, CreateConfiguredTargetError> {
    let artifact = rule_context.get_prerequisite_artifact(attr_name);
    rule_context.assert_no_errors()?;
    // If ruleContext.getPrerequisiteArtifact() set no errors, we know artifact != null
    let artifact = artifact.expect("verified by assertNoErrors");
    if !artifact.is_source_artifact() {
        let message = format!(
            "{} is not a source file and cannot be loaded in Starlark",
            format_derived_artifact(&artifact, repository_mapping)
        );
        rule_context.attribute_error(attr_name, &message);
        return Err(RuleErrorException::new(message).into());
    }
    Ok(artifact
        .get_owner()
        .expect("verifyNotNull(artifact.getOwner())"))
}

// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:183 StarlarkDocExtract.formatDerivedArtifact
fn format_derived_artifact(artifact: &Artifact, repository_mapping: &RepositoryMapping) -> String {
    blaze_util::check_argument!(!artifact.is_source_artifact());
    format!(
        "{} (generated by rule {})",
        artifact.get_repository_relative_path().get_path_string(),
        artifact
            .get_owner()
            .expect("a derived artifact has an owner")
            .get_display_form(Some(repository_mapping))
    )
}

/// Verifies that the module's transitive loads are a subset of the source artifacts in
/// files-to-build of the rule's deps.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:202 StarlarkDocExtract.verifyModuleDeps
fn verify_module_deps(
    rule_context: &RuleContext,
    module: &BzlLoadValue,
    repository_mapping: &RepositoryMapping,
) -> Result<(), CreateConfiguredTargetError> {
    // Note attr schema validates that deps are .bzl or .scl files.
    // bzl_library targets may contain both source artifacts and derived artifacts (e.g.
    // generated .bzl files for tests); only the source artifacts can be load()-ed by Bazel.
    let mut flattened_deps_source_artifacts: Vec<Artifact> = Vec::new();
    let mut flattened_deps_derived_artifacts: Vec<Artifact> = Vec::new();
    for dep in rule_context.get_prerequisites(DEPS_ATTR) {
        // TODO(https://github.com/bazelbuild/bazel/issues/18599): we are using FileProvider
        // instead of StarlarkLibraryInfo only because StarlarkLibraryInfo is defined in
        // bazel_skylib, not natively in Bazel.
        let file_provider = dep
            .get_provider::<FileProvider>()
            .expect("a dependency has a FileProvider");
        for artifact in file_provider.get_files_to_build().to_list().iter() {
            let set = if artifact.is_source_artifact() {
                &mut flattened_deps_source_artifacts
            } else {
                &mut flattened_deps_derived_artifacts
            };
            if !set.contains(artifact) {
                set.push(artifact.clone());
            }
        }
    }

    let known_modules: HashSet<Label> = flattened_deps_source_artifacts
        .iter()
        .map(|artifact| {
            artifact
                .get_owner()
                .expect("verifyNotNull(artifact.getOwner())")
        })
        .collect();
    let topmost_unknown_loads = get_topmost_unknown_loads(
        module.get_module_context(),
        &known_modules,
        repository_mapping,
    );

    if !topmost_unknown_loads.is_empty() {
        let mut error_message = format!(
            "missing bzl_library targets for Starlark module(s) {}",
            topmost_unknown_loads.join(", ")
        );
        if !flattened_deps_derived_artifacts.is_empty() {
            // TODO(arostovtsev): we ought to print only the derived artifacts having the same
            // root-relative path as topmostUnknownLoads.
            error_message.push_str(
                "\nNote the following are generated file(s) and cannot be loaded in Starlark: ",
            );
            error_message.push_str(
                &flattened_deps_derived_artifacts
                    .iter()
                    .map(|artifact| format_derived_artifact(artifact, repository_mapping))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        rule_context.attribute_error(DEPS_ATTR, &error_message);
        return Err(RuleErrorException::new(error_message).into());
    }
    Ok(())
}

/// Finds the topmost modules that are transitively loaded by the given module but not mentioned
/// in the given set of known modules, and returns these modules' display forms.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:254 StarlarkDocExtract.getTopmostUnknownLoads
fn get_topmost_unknown_loads(
    module_context: &BazelModuleContext,
    known_modules: &HashSet<Label>,
    repository_mapping: &RepositoryMapping,
) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    let mut visited: HashSet<Label> = HashSet::new();
    BazelModuleContext::visit_load_graph_recursively(&module_context.loads, &mut |loaded| {
        let label = loaded.label();
        if !visited.insert(label.clone()) {
            return false;
        }
        if !known_modules.contains(label) {
            unknown.push(label.get_display_form(Some(repository_mapping)));
            return false;
        }
        true
    });
    unknown
}

/// Returns the main repository's repo mapping value; `None` on a Skyframe restart.
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:273 StarlarkDocExtract.getMainRepositoryMappingValue
fn get_main_repository_mapping_value(
    rule_context: &RuleContext,
) -> Result<Option<MainRepositoryMapping>, CreateConfiguredTargetError> {
    let looked_up = blaze_analysis::skyframe_env::get_skyframe_env(|env| {
        env.get_value_or_throw_with(RepositoryMappingKey::key(MAIN.clone()), |e| {
            Some(e.to_string())
        })
    });
    match looked_up {
        Err(_) => Err(CreateConfiguredTargetError::Interrupted),
        Ok(Err(message)) => {
            rule_context.rule_error(&message);
            Err(RuleErrorException::new(message).into())
        }
        Ok(Ok(None)) => Ok(None),
        Ok(Ok(Some(value))) => Ok(Some(MainRepositoryMapping {
            repository_mapping: value
                .repository_mapping()
                .cloned()
                .expect("the main repository has a mapping"),
            associated_module_name: value.associated_module_name().map(str::to_owned),
        })),
    }
}

// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:293 StarlarkDocExtract.getModuleInfo
fn get_module_info(
    rule_context: &RuleContext,
    module: &BzlLoadValue,
    label_renderer: LabelRenderer,
) -> Result<blaze_protos::stardoc_output::ModuleInfo, CreateConfiguredTargetError> {
    let symbol_names: Vec<String> = match rule_context
        .attributes()
        .get(SYMBOL_NAMES_ATTR, Type::StringList)
    {
        Some(AttrValue::StringList(l)) => l,
        _ => Vec::new(),
    };
    // getWantedSymbolPredicate
    // Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:311 StarlarkDocExtract.getWantedSymbolPredicate
    let is_wanted = |name: &str| symbol_names.is_empty() || symbol_names.iter().any(|s| s == name);
    let mut module_info_extractor = ModuleInfoExtractor::new(&is_wanted, label_renderer);
    if bool_attr(rule_context, ALLOW_UNUSED_DOC_COMMENTS) {
        module_info_extractor = module_info_extractor.allow_unused_doc_comments();
    }
    let module_context = module.get_module_context();
    // BazelModuleContext.getDocCommentsMap() / getUnusedDocCommentLines(): the doc comments of
    // the compiled .bzl file.
    // PORT: Blaze's BazelModuleContext does not keep them; the file is read and its doc
    // comments are computed with the Java-faithful front end, as Java's compilation does.
    let doc_comments = compute_doc_comments(rule_context, module_context);
    let documented = DocumentedModule {
        module: module.get_module(),
        filename: Some(module_context.filename.clone()),
        label: Some(module_context.label().clone()),
    };
    match module_info_extractor.extract_from(
        &documented,
        &doc_comments.doc_comments_map,
        &doc_comments.unused_doc_comment_lines,
    ) {
        Ok(module_info) => Ok(module_info),
        Err(e) => {
            rule_context.rule_error(&e.message);
            Err(RuleErrorException::new(e.message).into())
        }
    }
}

/// The doc comments of the `src` file (Java `Program.getDocCommentsMap()` and
/// `getUnusedDocCommentLines()`, kept by `BazelModuleContext`).
fn compute_doc_comments(
    rule_context: &RuleContext,
    module_context: &BazelModuleContext,
) -> blaze_starlark::syntax::java::ProgramDocComments {
    use blaze_starlark::syntax::java::{
        JavaOptions, PredeclaredModule, parse_file, program_doc_comments,
    };
    let Some(src) = rule_context.get_prerequisite_artifact(SRC_ATTR) else {
        return Default::default();
    };
    let Ok(bytes) = std::fs::read(&*blaze_util::string_encoding::internal_to_platform(
        src.get_path().get_path_string(),
    )) else {
        return Default::default();
    };
    let source = blaze_util::string_encoding::latin1_decode(&bytes).into_owned();
    // The resolver's bindings (and so the doc comments map) do not depend on the predeclared
    // names; its errors are not read here.
    let parsed = parse_file(&module_context.filename, &source, JavaOptions::DEFAULT);
    let guarded = |_: &str| None;
    let predeclared = PredeclaredModule::new(Vec::new(), &guarded);
    program_doc_comments(&parsed, &predeclared, JavaOptions::DEFAULT)
}

// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:322 StarlarkDocExtract.createBinaryProtoOutput
fn create_binary_proto_output(
    rule_context: &RuleContext,
    module_info: &blaze_protos::stardoc_output::ModuleInfo,
) -> Artifact {
    let binary_proto_output =
        rule_context.get_implicit_output_artifact(&implicit_output(rule_context, BINARYPROTO_OUT));
    rule_context.register_action(Arc::new(BinaryFileWriteAction::new(
        rule_context.get_action_owner(),
        binary_proto_output.clone(),
        // moduleInfo.toByteArray()
        wrap(blaze_protos::JavaMessage::to_byte_array(module_info)),
        /* make_executable= */ false,
    )));
    binary_proto_output
}

// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtract.java:336 StarlarkDocExtract.createTextProtoOutput
fn create_text_proto_output(
    rule_context: &RuleContext,
    module_info: &blaze_protos::stardoc_output::ModuleInfo,
) -> Artifact {
    let text_proto_output =
        rule_context.get_implicit_output_artifact(&implicit_output(rule_context, TEXTPROTO_OUT));
    // TextFormat.printer().print(moduleInfo, textprotoBuilder)
    let textproto = blaze_protos::text_format::printer().print_to_string(module_info);
    rule_context.register_action(Arc::new(FileWriteAction::create_from_context(
        rule_context.get_action_owner(),
        text_proto_output.clone(),
        textproto,
        /* make_executable= */ false,
    )));
    text_proto_output
}

/// `ruleContext.getImplicitOutputArtifact(SafeImplicitOutputsFunction)`: the template's file name
/// for this rule (`fromTemplates("%{name}...")` substitutes the rule's name).
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java ImplicitOutputsFunction.fromTemplates
fn implicit_output(rule_context: &RuleContext, template: &str) -> String {
    template.replace("%{name}", rule_context.get_label().get_name())
}
