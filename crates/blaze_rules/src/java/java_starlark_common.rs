//! `_builtins.internal.java_common_internal_do_not_use` with analysis behaviour: ports of
//! `rules/java/JavaStarlarkCommon.java` (the Starlark methods rules_java reaches through
//! `java_common.internal_DO_NOT_USE()`) over `starlarkbuildapi/java/JavaCommonApi.java`'s
//! names and parameters.
//!
//! The loading-phase stand-in is `blaze_packages::builtins::java_common` (flag accessors only);
//! `crate::bazel_environment` replaces it with this value.

use std::fmt;
use std::sync::LazyLock;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configuredtargets::{get_rule_class_string, target_from_starlark};
use blaze_analysis::info::{info_provider_key, is_info};
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::builtin_restriction::fail_if_called_outside_default_allowlist;
use blaze_analysis::rule::depsets::{artifacts_to_depset, depset_value};
use blaze_analysis::starlark::starlark_rule_context::rule_context_from_ctx;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode as StrictDepsMode;
use blaze_packages::starlark_defs::provider::provider_view;
use blaze_starlark::ThreadContext;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StarlarkInfo;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::AllocList;
use starlark::values::none::NoneType;
use starlark::values::{
    Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, ValueLike, starlark_value,
};

use super::boot_class_path_info::BootClassPathInfo;
use super::java_compilation_helper::JavaCompilationHelper;
use super::java_helper;
use super::java_output::JavaCompileOutputs;
use super::java_plugin_info::JavaPluginInfo;
use super::java_semantics::{BAZEL_JAVA_SEMANTICS, JavaSemantics};
use super::java_target_attributes::JavaTargetAttributesBuilder;
use super::java_toolchain_provider::JavaToolchainProvider;
use crate::cpp::util::{errorf, is_sequence, sequence_cast_artifacts, sequence_elements};

/// A module that contains Starlark utilities for Java support.
///
/// PORT: Java holds a `JavaSemantics`; Bazel's is `BazelJavaSemantics.INSTANCE`, the only one.
// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:63 JavaStarlarkCommon
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaStarlarkCommon;

starlark_simple_value!(JavaStarlarkCommon);

impl fmt::Display for JavaStarlarkCommon {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (a @StarlarkBuiltin
    // named "java_common").
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<java_common>")
    }
}

/// `_builtins.internal.java_common_internal_do_not_use`
/// (`new JavaStarlarkCommon(BazelJavaSemantics.INSTANCE)`), frozen, for the analysis-capable
/// builtins environment.
// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:64 JavaRules.init
pub fn java_common_internal_value() -> OwnedFrozenValue {
    blaze_packages::builtins::env::freeze_value(|m| m.heap().alloc(JavaStarlarkCommon))
}

#[starlark_value(type = "java_common")]
impl<'v> StarlarkValue<'v> for JavaStarlarkCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("java_common", java_common_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:602 JavaCommonApi.getJavaToolchainProvider
    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:614 JavaCommonApi.getJavaRuntimeProvider
    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:623 JavaCommonApi.getBootClassPathInfo
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: the default methods of JavaCommonApi throw UnsupportedOperationException
            // (they exist solely for documentation).
            "JavaToolchainInfo" | "JavaRuntimeInfo" | "BootClassPathInfo" => {
                panic!("UnsupportedOperationException: java_common.{attribute}")
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            "JavaToolchainInfo" | "JavaRuntimeInfo" | "BootClassPathInfo"
        )
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "BootClassPathInfo".to_owned(),
            "JavaRuntimeInfo".to_owned(),
            "JavaToolchainInfo".to_owned(),
        ]
    }
}

fn semantics() -> &'static dyn JavaSemantics {
    &BAZEL_JAVA_SEMANTICS
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:72 JavaStarlarkCommon.getStrictDepsMode
fn get_strict_deps_mode(strict_deps_mode: &str) -> StrictDepsMode {
    match strict_deps_mode {
        "OFF" => StrictDepsMode::OFF,
        "ERROR" | "DEFAULT" => StrictDepsMode::ERROR,
        "WARN" => StrictDepsMode::WARN,
        _ => panic!(
            "IllegalArgumentException: StrictDepsMode {} not allowed. Only OFF and ERROR values \
             are accepted.",
            strict_deps_mode
        ),
    }
}

/// Java `Ascii.toUpperCase(string)`: only ASCII letters change.
fn ascii_to_upper_case(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_uppercase()).collect()
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:90 JavaStarlarkCommon.checkJavaToolchainIsDeclaredOnRule
fn check_java_toolchain_is_declared_on_rule(rule_context: &RuleContext) -> starlark::Result<()> {
    let toolchain_type = semantics().get_java_toolchain_type();
    let label = Label::parse_canonical(&toolchain_type).map_err(|e| errorf(e.to_string()))?;
    let toolchain_info = rule_context.get_toolchain_info(&label);
    if toolchain_info.is_none() {
        let rule_location = rule_context.get_rule().get_location().to_string();
        let rule_class = rule_context.get_rule().get_rule_class_object().get_name();
        return Err(errorf(format!(
            "Rule '{}' in '{}' must declare '{}' toolchain in order to use java_common. See \
             https://github.com/bazelbuild/bazel/issues/18970.",
            rule_class, rule_location, toolchain_type
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:291 JavaStarlarkCommon.checkPrivateAccess
fn check_private_access(eval: &Evaluator) -> starlark::Result<()> {
    fail_if_called_outside_default_allowlist(eval)
}

/// `(Label) x` of a `Label` parameter.
fn label_param(v: Value, param: &str) -> starlark::Result<Label> {
    match v.downcast_ref::<LabelValue>() {
        Some(l) => Ok(l.label().clone()),
        None => Err(errorf(format!(
            "got value of type '{}', want 'Label' for parameter {param}",
            v.get_type()
        ))),
    }
}

/// `(Artifact) x` of a File parameter.
fn artifact_param(v: Value, param: &str) -> starlark::Result<Artifact> {
    artifact_from_value(v).ok_or_else(|| {
        errorf(format!(
            "got value of type '{}', want 'File' for parameter {param}",
            v.get_type()
        ))
    })
}

/// `x == Starlark.NONE ? null : (Artifact) x`.
fn noneable_artifact(v: Value, param: &str) -> starlark::Result<Option<Artifact>> {
    if v.is_none() {
        Ok(None)
    } else {
        artifact_param(v, param).map(Some)
    }
}

/// `depset.getSet(Artifact.class)` of a `Depset` parameter.
fn depset_artifacts(v: Value, param: &str) -> starlark::Result<NestedSet<Artifact>> {
    blaze_analysis::rule::depsets::depset_to_artifacts(v, param)
}

/// `Depset.cast(x, String.class, what)`.
fn depset_strings(v: Value, param: &str) -> starlark::Result<NestedSet<String>> {
    blaze_analysis::rule::depsets::depset_to_strings(v, param)
}

/// The rule context of `ctx` and the evaluator's heap, as Java's `ctx.getRuleContext()`.
fn rule_context<'v>(ctx: Value<'v>) -> starlark::Result<std::cell::Ref<'v, RuleContext>> {
    rule_context_from_ctx(ctx)
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:295 JavaStarlarkCommon.getDynamicLibrariesForLinking
fn get_dynamic_libraries_for_linking<'v>(libraries: &[Value<'v>]) -> Vec<Artifact> {
    let mut dynamic_libraries_for_linking_builder = Vec::new();
    for library_to_link in libraries {
        let info = StarlarkInfo::from_value(*library_to_link);
        let get = |k: &str| info.and_then(|i| i.get_field(k));
        if let Some(artifact) = get("interface_library").and_then(artifact_from_value) {
            dynamic_libraries_for_linking_builder.push(artifact);
        } else if let Some(artifact) = get("dynamic_library").and_then(artifact_from_value) {
            dynamic_libraries_for_linking_builder.push(artifact);
        }
    }
    dynamic_libraries_for_linking_builder
}

/// `CppFileTypes.SHARED_LIBRARY.matches(name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:243 CppFileTypes.SHARED_LIBRARY
fn is_shared_library(name: &str) -> bool {
    [".so", ".dylib", ".dll", ".pyd", ".wasm", ".tgt", ".vpi"]
        .iter()
        .any(|e| name.ends_with(e))
}

/// `CppFileTypes.INTERFACE_SHARED_LIBRARY.matches(name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:247 CppFileTypes.INTERFACE_SHARED_LIBRARY
fn is_interface_shared_library(name: &str) -> bool {
    [".ifso", ".tbd", ".lib", ".dll.a"]
        .iter()
        .any(|e| name.ends_with(e))
}

/// `CppFileTypes.VERSIONED_SHARED_LIBRARY.matches(name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:258 CppFileTypes.VERSIONED_SHARED_LIBRARY
fn is_versioned_shared_library(path: &str) -> bool {
    // Because regex matching can be slow, we first do a quick check for ".so." and ".dylib."
    // substring before risking the full-on regex match.
    if !path.contains(".so.") && !path.contains(".dylib.") {
        return false;
    }
    // Java: Pattern "^.+\\.((so)|(dylib))(\\.\\d\\w*)+$" (ASCII \d and \w).
    static PATTERN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^(?s:.)+\.((so)|(dylib))(\.[0-9][A-Za-z0-9_]*)+$").unwrap()
    });
    // PORT: Java's `.` does not match line terminators; paths do not contain them.
    PATTERN.is_match(path)
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:372 JavaStarlarkCommon.printableType
fn printable_type(elem: Value) -> String {
    if let Some(info) = StarlarkInfo::from_value(elem) {
        if super::provider_util::is_starlark_info_with_schema(elem)
            && let Some(view) = provider_view(info.provider())
            && view.key.is_some()
        {
            return blaze_packages::starlark_defs::provider::StarlarkProvider::from_value(
                info.provider(),
            )
            .map(|p| p.get_printable_name())
            .unwrap_or_else(|| elem.get_type().to_owned());
        }
        return elem.get_type().to_owned();
    }
    // Java: NativeInfo -> nativeInfo.getProvider().getPrintableName(): the builtin's name.
    if let Some(key) = info_provider_key(elem) {
        return key.to_string();
    }
    elem.get_type().to_owned()
}

/// Java `Provider.getKey()` of a provider value (`None` for a non-provider; an unexported
/// Starlark provider has no key: Java's `getKey()` throws).
fn provider_key_of(provider: Value) -> Option<Option<blaze_packages::rule_class::ProviderKey>> {
    provider_view(provider).map(|v| v.key)
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:427 JavaStarlarkCommon.isInstanceOfProvider
fn is_instance_of_provider(
    obj: Value,
    provider_key: &blaze_packages::rule_class::ProviderKey,
) -> bool {
    match info_provider_key(obj) {
        Some(k) => &k == provider_key,
        None => false,
    }
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:116 JavaStarlarkCommon.createHeaderCompilationAction
#[allow(clippy::too_many_arguments)]
fn create_header_compilation_action<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    ctx: Value<'v>,
    toolchain: Value<'v>,
    header_jar: Value<'v>,
    header_deps_proto: Value<'v>,
    plugin_info: Value<'v>,
    source_files: Value<'v>,
    source_jars: Value<'v>,
    compile_time_classpath: Value<'v>,
    direct_jars: Value<'v>,
    boot_class_path_unchecked: Value<'v>,
    compile_time_java_deps: Value<'v>,
    javac_opts: Value<'v>,
    strict_deps_mode: &str,
    target_label: Value<'v>,
    injecting_rule_kind: Value<'v>,
    enable_direct_classpath: bool,
    additional_inputs: Value<'v>,
    header_compilation_jar: Value<'v>,
    header_compilation_direct_deps: Value<'v>,
) -> starlark::Result<()> {
    let heap = eval.heap();
    let rule_context = rule_context(ctx)?;
    check_java_toolchain_is_declared_on_rule(&rule_context)?;
    let mut attributes_builder = JavaTargetAttributesBuilder::new();
    attributes_builder
        .add_source_jars(&sequence_cast_artifacts(source_jars, "source_jars", heap)?)
        .add_source_files(
            depset_artifacts(source_files, "source_files")?
                .to_list()
                .iter(),
        )
        .add_direct_jars(&depset_artifacts(direct_jars, "direct_jars")?)
        .add_header_compilation_direct_jars(&depset_artifacts(
            header_compilation_direct_deps,
            "header_compilation_direct_deps",
        )?)
        .set_compile_time_class_path_entries_with_prepended_direct_jars(&depset_artifacts(
            compile_time_classpath,
            "compilation_classpath",
        )?)
        .set_strict_java_deps(get_strict_deps_mode(&ascii_to_upper_case(strict_deps_mode)))
        .set_target_label(label_param(target_label, "target_label")?)
        .set_injecting_rule_kind(if injecting_rule_kind.is_none() {
            None
        } else {
            Some(
                injecting_rule_kind
                    .unpack_str()
                    .unwrap_or_else(|| panic!("ClassCastException: injecting_rule_kind"))
                    .to_owned(),
            )
        })
        .add_plugin(&JavaPluginInfo::wrap(plugin_info, heap)?)
        .add_compile_time_dependency_artifacts(&depset_artifacts(
            compile_time_java_deps,
            "compile_time_java_deps",
        )?);
    if is_info(boot_class_path_unchecked) {
        let boot_class_path_info = BootClassPathInfo::wrap(boot_class_path_unchecked, heap);
        if !boot_class_path_info.is_empty()? {
            attributes_builder.set_boot_class_path(boot_class_path_info)?;
        }
    }
    let mut compilation_helper = JavaCompilationHelper::new(
        &rule_context,
        semantics(),
        java_helper::tokenize_java_options_nested_set(&depset_strings(javac_opts, "javac_opts")?),
        attributes_builder,
        JavaToolchainProvider::wrap(toolchain, heap)?,
        sequence_cast_artifacts(additional_inputs, "additional_inputs", heap)?,
    );
    compilation_helper.enable_direct_classpath(enable_direct_classpath);
    // Java: (Artifact) headerCompilationJar; a None value is a null Artifact (the builder's
    // checkNotNull then fails as in Java).
    let header_compilation_jar =
        noneable_artifact(header_compilation_jar, "header_compilation_jar")?.unwrap_or_else(|| {
            panic!("NullPointerException: headerCompilationOutputJar must not be null")
        });
    compilation_helper.create_header_compilation_action(
        eval,
        &artifact_param(header_jar, "compile_jar")?,
        &header_compilation_jar,
        &artifact_param(header_deps_proto, "compile_deps_proto")?,
    )
}

// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:177 JavaStarlarkCommon.createCompilationAction
#[allow(clippy::too_many_arguments)]
fn create_compilation_action<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    ctx: Value<'v>,
    java_toolchain: Value<'v>,
    output: Value<'v>,
    manifest_proto: Value<'v>,
    plugin_info: Value<'v>,
    compile_time_classpath: Value<'v>,
    direct_jars: Value<'v>,
    boot_class_path_unchecked: Value<'v>,
    java_builder_jvm_flags: Value<'v>,
    compile_time_java_deps: Value<'v>,
    javac_opts: Value<'v>,
    strict_deps_mode: &str,
    target_label: Value<'v>,
    deps_proto: Value<'v>,
    gen_class: Value<'v>,
    gen_source: Value<'v>,
    native_header: Value<'v>,
    source_files: Value<'v>,
    source_jars: Value<'v>,
    resources: Value<'v>,
    resource_jars: Value<'v>,
    classpath_resources: Value<'v>,
    sourcepath: Value<'v>,
    injecting_rule_kind: Value<'v>,
    enable_jspecify: bool,
    enable_direct_classpath: bool,
    additional_inputs: Value<'v>,
    additional_outputs: Value<'v>,
    extra_args: Value<'v>,
) -> starlark::Result<()> {
    let heap = eval.heap();
    let rule_context = rule_context(ctx)?;
    check_java_toolchain_is_declared_on_rule(&rule_context)?;
    let outputs = JavaCompileOutputs::builder()
        .output(artifact_param(output, "output")?)
        .deps_proto(noneable_artifact(deps_proto, "deps_proto")?)
        .gen_class(noneable_artifact(gen_class, "gen_class")?)
        .gen_source(noneable_artifact(gen_source, "gen_source")?)
        .native_header(noneable_artifact(native_header, "native_header_jar")?)
        .manifest_proto(Some(artifact_param(manifest_proto, "manifest_proto")?))
        .build();
    let extra_args_elems = if is_sequence(extra_args) {
        sequence_elements(extra_args, heap)?
    } else {
        return Err(errorf(format!(
            "for extra_args, got {}, want sequence",
            extra_args.get_type()
        )));
    };
    if !extra_args_elems.is_empty() {
        // Java: extraCommandLineArgs.add(args.build(mainRepoMapping)) for each Args.
        blaze_util::unported!(
            "JavaStarlarkCommon.createCompilationAction",
            "extra_args (Args.build of a Starlark Args object outside an action)"
        );
    }
    let source_files_list: Vec<Artifact> = if source_files.is_none() {
        Vec::new()
    } else {
        depset_artifacts(source_files, "sources")?
            .to_list()
            .to_vec()
    };
    let mut attributes_builder = JavaTargetAttributesBuilder::new();
    attributes_builder
        .add_source_jars(&sequence_cast_artifacts(source_jars, "source_jars", heap)?)
        .add_source_files(source_files_list.iter())
        .add_direct_jars(&depset_artifacts(direct_jars, "direct_jars")?)
        .set_compile_time_class_path_entries_with_prepended_direct_jars(&depset_artifacts(
            compile_time_classpath,
            "compilation_classpath",
        )?)
        .add_class_path_resources(&sequence_cast_artifacts(
            classpath_resources,
            "classpath_resources",
            heap,
        )?)
        .set_strict_java_deps(get_strict_deps_mode(&ascii_to_upper_case(strict_deps_mode)))
        .set_target_label(label_param(target_label, "target_label")?)
        .set_injecting_rule_kind(if injecting_rule_kind.is_none() {
            None
        } else {
            Some(
                injecting_rule_kind
                    .unpack_str()
                    .unwrap_or_else(|| panic!("ClassCastException: injecting_rule_kind"))
                    .to_owned(),
            )
        })
        .set_source_path(sequence_cast_artifacts(sourcepath, "source_path", heap)?)
        .add_plugin(&JavaPluginInfo::wrap(plugin_info, heap)?)
        .add_additional_outputs(
            sequence_cast_artifacts(additional_outputs, "additional_outputs", heap)?.iter(),
        );
    if is_info(boot_class_path_unchecked) {
        let boot_class_path_info = BootClassPathInfo::wrap(boot_class_path_unchecked, heap);
        if !boot_class_path_info.is_empty()? {
            attributes_builder.set_boot_class_path(boot_class_path_info)?;
        }
    }
    for resource in sequence_cast_artifacts(resources, "resources", heap)? {
        attributes_builder.add_resource(
            java_helper::get_java_resource_path(semantics(), &rule_context, &resource),
            resource,
        );
    }
    attributes_builder.add_resource_jars(&if resource_jars.is_none() {
        NestedSetBuilder::empty_set(Order::STABLE_ORDER)
    } else {
        depset_artifacts(resource_jars, "resource_jars")?
    });
    attributes_builder.add_compile_time_dependency_artifacts(&depset_artifacts(
        compile_time_java_deps,
        "compile_time_java_deps",
    )?);
    let mut compilation_helper = JavaCompilationHelper::new(
        &rule_context,
        semantics(),
        java_helper::tokenize_java_options_nested_set(&depset_strings(javac_opts, "javac_opts")?),
        attributes_builder,
        JavaToolchainProvider::wrap(java_toolchain, heap)?,
        sequence_cast_artifacts(additional_inputs, "additional_inputs", heap)?,
    );
    compilation_helper.java_builder_jvm_flags(depset_strings(
        java_builder_jvm_flags,
        "javabuilder_jvm_flags",
    )?);
    compilation_helper.enable_jspecify(enable_jspecify);
    compilation_helper.enable_direct_classpath(enable_direct_classpath);
    compilation_helper.set_extra_command_line_args(Vec::new());
    compilation_helper.create_compile_action(eval, outputs)
}

#[starlark_module]
fn java_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:64 JavaCommonApi.mergeJavaProviders
    fn merge<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] providers: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let _ = (this, providers);
        // Java: default method; throws UnsupportedOperationException.
        panic!("UnsupportedOperationException: java_common.merge")
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:109 JavaCommonApi.packSources
    fn pack_sources<'v>(
        this: Value<'v>,
        #[starlark(require = named)] actions: Value<'v>,
        #[starlark(require = named, default = NoneType)] output_source_jar: Value<'v>,
        #[starlark(require = named)] sources: Option<Value<'v>>,
        #[starlark(require = named)] source_jars: Option<Value<'v>>,
        #[starlark(require = named)] java_toolchain: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let _ = (
            this,
            actions,
            output_source_jar,
            sources,
            source_jars,
            java_toolchain,
        );
        panic!("UnsupportedOperationException: java_common.pack_sources")
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:150 JavaCommonApi.stampJar
    fn stamp_jar<'v>(
        this: Value<'v>,
        #[starlark(require = named)] actions: Value<'v>,
        #[starlark(require = named)] jar: Value<'v>,
        #[starlark(require = named)] target_label: Value<'v>,
        #[starlark(require = named)] java_toolchain: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let _ = (this, actions, jar, target_label, java_toolchain);
        panic!("UnsupportedOperationException: java_common.stamp_jar")
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:186 JavaCommonApi.runIjar
    fn run_ijar<'v>(
        this: Value<'v>,
        #[starlark(require = named)] actions: Value<'v>,
        #[starlark(require = named)] jar: Value<'v>,
        #[starlark(require = named, default = NoneType)] target_label: Value<'v>,
        #[starlark(require = named)] java_toolchain: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let _ = (this, actions, jar, target_label, java_toolchain);
        panic!("UnsupportedOperationException: java_common.run_ijar")
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:409 JavaCommonApi.createJavaCompileAction
    fn compile<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
        #[starlark(kwargs)] kwargs: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let _ = (this, ctx, kwargs);
        panic!("UnsupportedOperationException: java_common.compile")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:116 JavaStarlarkCommon.createHeaderCompilationAction
    fn create_header_compilation_action<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
        #[starlark(require = pos)] java_toolchain: Value<'v>,
        #[starlark(require = pos)] compile_jar: Value<'v>,
        #[starlark(require = pos)] compile_deps_proto: Value<'v>,
        #[starlark(require = pos)] plugin_info: Value<'v>,
        #[starlark(require = pos)] source_files: Value<'v>,
        #[starlark(require = pos)] source_jars: Value<'v>,
        #[starlark(require = pos)] compilation_classpath: Value<'v>,
        #[starlark(require = pos)] direct_jars: Value<'v>,
        #[starlark(require = pos)] bootclasspath: Value<'v>,
        #[starlark(require = pos)] compile_time_java_deps: Value<'v>,
        #[starlark(require = pos)] javac_opts: Value<'v>,
        #[starlark(require = pos)] strict_deps_mode: &str,
        #[starlark(require = pos)] target_label: Value<'v>,
        #[starlark(require = pos)] injecting_rule_kind: Value<'v>,
        #[starlark(require = pos)] enable_direct_classpath: bool,
        #[starlark(require = pos)] additional_inputs: Value<'v>,
        header_compilation_jar: Value<'v>,
        header_compilation_direct_deps: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        create_header_compilation_action(
            eval,
            ctx,
            java_toolchain,
            compile_jar,
            compile_deps_proto,
            plugin_info,
            source_files,
            source_jars,
            compilation_classpath,
            direct_jars,
            bootclasspath,
            compile_time_java_deps,
            javac_opts,
            strict_deps_mode,
            target_label,
            injecting_rule_kind,
            enable_direct_classpath,
            additional_inputs,
            header_compilation_jar,
            header_compilation_direct_deps,
        )?;
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:177 JavaStarlarkCommon.createCompilationAction
    fn create_compilation_action<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
        #[starlark(require = pos)] java_toolchain: Value<'v>,
        #[starlark(require = pos)] output: Value<'v>,
        #[starlark(require = pos)] manifest_proto: Value<'v>,
        #[starlark(require = pos)] plugin_info: Value<'v>,
        #[starlark(require = pos)] compilation_classpath: Value<'v>,
        #[starlark(require = pos)] direct_jars: Value<'v>,
        #[starlark(require = pos)] bootclasspath: Value<'v>,
        #[starlark(require = pos)] javabuilder_jvm_flags: Value<'v>,
        #[starlark(require = pos)] compile_time_java_deps: Value<'v>,
        #[starlark(require = pos)] javac_opts: Value<'v>,
        #[starlark(require = pos)] strict_deps_mode: &str,
        #[starlark(require = pos)] target_label: Value<'v>,
        #[starlark(default = NoneType)] deps_proto: Value<'v>,
        #[starlark(default = NoneType)] gen_class: Value<'v>,
        #[starlark(default = NoneType)] gen_source: Value<'v>,
        #[starlark(default = NoneType)] native_header_jar: Value<'v>,
        #[starlark(default = NoneType)] sources: Value<'v>,
        source_jars: Option<Value<'v>>,
        resources: Option<Value<'v>>,
        #[starlark(default = NoneType)] resource_jars: Value<'v>,
        classpath_resources: Option<Value<'v>>,
        sourcepath: Option<Value<'v>>,
        #[starlark(default = NoneType)] injecting_rule_kind: Value<'v>,
        #[starlark(default = true)] enable_jspecify: bool,
        #[starlark(default = true)] enable_direct_classpath: bool,
        additional_inputs: Option<Value<'v>>,
        additional_outputs: Option<Value<'v>>,
        #[starlark(require = named)] extra_args: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        let empty = eval.heap().alloc(AllocList::EMPTY);
        create_compilation_action(
            eval,
            ctx,
            java_toolchain,
            output,
            manifest_proto,
            plugin_info,
            compilation_classpath,
            direct_jars,
            bootclasspath,
            javabuilder_jvm_flags,
            compile_time_java_deps,
            javac_opts,
            strict_deps_mode,
            target_label,
            deps_proto,
            gen_class,
            gen_source,
            native_header_jar,
            sources,
            source_jars.unwrap_or(empty),
            resources.unwrap_or(empty),
            resource_jars,
            classpath_resources.unwrap_or(empty),
            sourcepath.unwrap_or(empty),
            injecting_rule_kind,
            enable_jspecify,
            enable_direct_classpath,
            additional_inputs.unwrap_or(empty),
            additional_outputs.unwrap_or(empty),
            extra_args.unwrap_or(empty),
        )?;
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:592 JavaCommonApi.getDefaultJavacOpts
    fn default_javac_opts<'v>(
        this: Value<'v>,
        #[starlark(require = named)] java_toolchain: Value<'v>,
        #[starlark(require = named, default = false)] as_depset: bool,
    ) -> anyhow::Result<NoneType> {
        let _ = (this, java_toolchain, as_depset);
        // method exists solely for documentation
        panic!("UnsupportedOperationException: java_common.default_javac_opts")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:277 JavaStarlarkCommon.getTargetKind
    fn target_kind<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] target: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        let _ = this;
        check_private_access(eval)?;
        let Some(mut target) = target_from_starlark(target) else {
            return Ok(String::new());
        };
        if let ConfiguredTarget::Merged(merged) = &target {
            target = merged.get_base_configured_target().clone();
        }
        target = target.get_actual();
        Ok(get_rule_class_string(&target))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:309 JavaStarlarkCommon.collectNativeLibsDirs
    fn collect_native_deps_dirs<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] libraries: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        check_private_access(eval)?;
        let depset = blaze_starlark::depset::Depset::from_value(libraries).ok_or_else(|| {
            errorf(format!(
                "got value of type '{}', want 'depset' for parameter libraries",
                libraries.get_type()
            ))
        })?;
        let elements = depset.to_list()?;
        let native_libraries = get_dynamic_libraries_for_linking(&elements);
        let mut unique_dirs: Vec<String> = Vec::new();
        for native_library in native_libraries {
            let name = native_library.get_filename();
            if is_interface_shared_library(name) {
                continue;
            }
            if !(is_shared_library(name) || is_versioned_shared_library(name)) {
                panic!(
                    "IllegalArgumentException: not a shared library :{}",
                    native_library.pretty_print()
                );
            }
            let dir = native_library
                .get_runfiles_path()
                .get_parent_directory()
                .map(|p| p.get_path_string().to_string())
                .unwrap_or_else(|| panic!("NullPointerException: getParentDirectory"));
            if !unique_dirs.contains(&dir) {
                unique_dirs.push(dir);
            }
        }
        let heap = eval.heap();
        let values: Vec<Value<'v>> = unique_dirs
            .iter()
            .map(|s| heap.alloc_str(s).to_value())
            .collect();
        Ok(starlark::values::list::alloc_immutable_list(heap, &values))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:336 JavaStarlarkCommon.getRuntimeClasspathForArchive
    fn get_runtime_classpath_for_archive<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] jars: Value<'v>,
        #[starlark(require = pos)] excluded_jars: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        check_private_access(eval)?;
        let excluded =
            blaze_starlark::depset::Depset::from_value(excluded_jars).ok_or_else(|| {
                errorf(format!(
                    "got value of type '{}', want 'depset' for parameter excluded_jars",
                    excluded_jars.get_type()
                ))
            })?;
        if excluded.is_empty() {
            return Ok(jars);
        }
        let excluded_set = depset_artifacts(excluded_jars, "excluded_jars")?.to_set();
        let runtime_classpath = depset_artifacts(jars, "jars")?;
        let filtered: Vec<Artifact> = runtime_classpath
            .to_list()
            .iter()
            .filter(|a| !excluded_set.contains(*a))
            .cloned()
            .collect();
        Ok(depset_value(&artifacts_to_depset(&NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            filtered,
        ))))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:354 JavaStarlarkCommon.checkProviderInstances
    fn check_provider_instances<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] providers: Value<'v>,
        #[starlark(require = pos)] what: &str,
        #[starlark(require = pos)] provider_type: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        check_private_access(eval)?;
        match provider_key_of(provider_type) {
            Some(key) => {
                let key = key.unwrap_or_else(|| {
                    panic!("IllegalStateException: getKey() of an unexported provider")
                });
                let elems = sequence_elements(providers, eval.heap())?;
                for (i, elem) in elems.into_iter().enumerate() {
                    if !is_instance_of_provider(elem, &key) {
                        let printable_name =
                            blaze_packages::starlark_defs::provider::StarlarkProvider::from_value(
                                provider_type,
                            )
                            .map(|p| p.get_printable_name())
                            .unwrap_or_else(|| key.to_string());
                        return Err(errorf(format!(
                            "at index {} of {}, got element of type {}, want {}",
                            i,
                            what,
                            printable_type(elem),
                            printable_name
                        )));
                    }
                }
                Ok(NoneType)
            }
            None => Err(errorf(format!(
                "wanted Provider, got {}",
                provider_type.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:383 JavaStarlarkCommon.isLegacyGoogleApiEnabled
    fn _google_legacy_api_enabled<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        check_private_access(eval)?;
        Ok(ThreadContext::get(eval)
            .map_err(|e| errorf(e.to_string()))?
            .semantics
            .experimental_google_legacy_api)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:104 JavaStarlarkCommon.checkJavaToolchainIsDeclaredOnRuleForStarlark
    fn _check_java_toolchain_is_declared_on_rule<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        check_private_access(eval)?;
        let handle = crate::cpp::wrapped_action_factory::actions_rule_context(
            actions,
            "_check_java_toolchain_is_declared_on_rule",
            "actions",
        )?;
        let rule_context = handle.borrow();
        check_java_toolchain_is_declared_on_rule(&rule_context)?;
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:389 JavaStarlarkCommon.isJavaInfoMergeRuntimeModuleFlagsEnabled
    fn _incompatible_java_info_merge_runtime_module_flags<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        check_private_access(eval)?;
        Ok(ThreadContext::get(eval)
            .map_err(|e| errorf(e.to_string()))?
            .semantics
            .incompatible_java_info_merge_runtime_module_flags)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:398 JavaStarlarkCommon.incompatibleDisableNonExecutableJavaBinary
    fn incompatible_disable_non_executable_java_binary<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        Ok(ThreadContext::get(eval)
            .map_err(|e| errorf(e.to_string()))?
            .semantics
            .incompatible_disable_non_executable_java_binary)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:405 JavaStarlarkCommon.expandJavaOpts
    fn expand_java_opts<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
        #[starlark(require = pos)] attr: &str,
        #[starlark(require = named)] tokenize: bool,
        #[starlark(require = named, default = false)] exec_paths: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let rule_context = rule_context(ctx)?;
        let base = rule_context.get_expander();
        let expander = if exec_paths {
            base.with_exec_locations(IndexMap::new())
        } else {
            base.with_data_locations()
        };
        let result = if tokenize {
            expander.tokenized_attribute(attr)
        } else {
            expander.list_attribute(attr)
        };
        let heap = eval.heap();
        let values: Vec<Value<'v>> = result
            .iter()
            .map(|s| heap.alloc_str(s).to_value())
            .collect();
        Ok(starlark::values::list::alloc_immutable_list(heap, &values))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:422 JavaStarlarkCommon.tokenizeJavacOpts
    fn tokenize_javacopts<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] opts: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let heap = eval.heap();
        let opts: Vec<String> = if opts.is_none() {
            Vec::new()
        } else {
            crate::cpp::util::sequence_cast_strings(opts, "opts", heap)?
        };
        let tokens = java_helper::tokenize_java_options(&opts);
        let values: Vec<Value<'v>> = tokens
            .iter()
            .map(|s| heap.alloc_str(s).to_value())
            .collect();
        Ok(starlark::values::list::alloc_immutable_list(heap, &values))
    }
}
