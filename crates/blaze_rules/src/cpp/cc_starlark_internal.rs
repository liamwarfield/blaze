//! `cc_internal` (Java `CcStarlarkInternal`): the private natives rules_cc reaches through
//! `cc_common.internal_DO_NOT_USE()`, with Bazel's exact names and parameters.
//!
//! The Starlark-independent bodies are in `blaze_cc_features::cc_starlark_internal`; this module
//! does the argument casting, the rule-context work and the action registration.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::starlark::args::RegisteredArgs;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_actions::rules::cpp::solib_symlink_action as solib;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::analysis_utils::get_unique_directory;
use blaze_analysis::rule::artifact_roots::to_actions_root;
use blaze_analysis::starlark::RuleContextHandle;
use blaze_analysis::starlark::starlark_rule_context::{
    make_label_map, starlark_rule_context_from_value,
};
use blaze_analysis::starlark::thread_payload::register_action;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cc_features::cc_starlark_internal as body;
use blaze_cc_features::cc_toolchain_variables::CcToolchainVariablesRef;
use blaze_starlark::label::LabelValue;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::none::NoneType;
use starlark::values::tuple::{AllocTuple, TupleRef, UnpackTuple};
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use super::cc_artifact::to_actions_param_file_info;
use super::cc_compilation_context::CcCompilationContext;
use super::cc_toolchain_provider::CcToolchainProvider;
use super::cpp_compile_action::{
    BuildAndVerifyError, CcToolchainCompileData, CppCompileActionBuilder,
};
use super::cpp_configuration::cpp_configuration_param;
use super::header_info::{HeaderInfo, HeaderInfoData, header_info_of};
use super::link_command_line::LinkCommandLineAdapter;
use super::starlark_values::{
    CcToolchainFeaturesValue, CcToolchainVariablesValue, feature_configuration_param,
    to_cc_starlark_info, variables_param,
};
use super::util::{
    allowlist_entry_allows, artifact_param, configuration_param, depset_cast_artifacts,
    depset_noneable_cast_artifacts, errorf, is_sequence, null_if_none_artifact,
    sequence_cast_artifacts, sequence_cast_strings, sequence_elements,
};
use super::variables::{cast_vars, eval_error};
use super::wrapped_action_factory::{
    WrappedStarlarkActionFactory, actions_rule_context, new_action_construction,
};

/// Java `CcStarlarkInternal`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:87 CcStarlarkInternal
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct CcStarlarkInternal;

starlark_simple_value!(CcStarlarkInternal);

impl fmt::Display for CcStarlarkInternal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<cc_internal>")
    }
}

#[starlark_value(type = "cc_internal")]
impl<'v> StarlarkValue<'v> for CcStarlarkInternal {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("cc_internal", cc_internal_methods);
        Some(RES.methods())
    }
}

// -------------------------------------------------------------------------------------------
// Parameter helpers

/// The rule context behind a `ctx` parameter (Java `starlarkRuleContext.getRuleContext()`).
fn ctx_rule_context(
    ctx: Value,
    function: &str,
    param: &str,
) -> starlark::Result<RuleContextHandle> {
    match starlark_rule_context_from_value(ctx) {
        Some(c) => Ok(c.get_rule_context()),
        None => Err(errorf(format!(
            "in call to {function}(), parameter '{param}' got value of type '{}', want 'ctx'",
            ctx.get_type()
        ))),
    }
}

/// A `StarlarkList.immutableCopyOf(...)` result (or the immutable `"[]"` parameter default):
/// an immutable list.
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
fn immutable_string_list<'v>(heap: Heap<'v>, items: &[String]) -> Value<'v> {
    let values: Vec<Value<'v>> = items.iter().map(|s| heap.alloc_str(s).to_value()).collect();
    starlark::values::list::alloc_immutable_list(heap, &values)
}

/// An `ImmutableList<String>` returned by a `@StarlarkMethod`: `MethodDescriptor.call` converts
/// it with `Starlark.fromJava(result, thread.mutability())` = `StarlarkList.copyOf`, a new
/// mutable list per call.
// Java: net/starlark/java/eval/Starlark.java:214 Starlark.fromJava
fn mutable_string_list<'v>(heap: Heap<'v>, items: &[String]) -> Value<'v> {
    heap.alloc(starlark::values::list::AllocList(
        items.iter().map(|s| heap.alloc_str(s).to_value()),
    ))
}

/// `TargetUtils.getExecutionInfo(rule, allowTagsPropagation)` in Java's iteration order
/// (`ImmutableMap.copyOf` of a `HashMap` filled in `tags` order), which
/// `CppCompileActionBuilder.addExecutionInfo` keeps in its `LinkedHashMap`
/// (rules/cpp/CppCompileActionBuilder.java:71).
// PORT: java-hash-order (see `crate::generic::target_utils`).
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:264 TargetUtils.getExecutionInfo
fn target_execution_info(rule_context: &RuleContext) -> IndexMap<String, String> {
    crate::generic::target_utils::get_execution_info_with_propagation(
        rule_context.get_rule(),
        rule_context.is_allow_tags_propagation(),
    )
}

/// `Module.ofInnermostEnclosingStarlarkFunction(thread, depth)` and the allowlist check of
/// `check_private_api`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:111 CcStarlarkInternal.checkPrivateApi
fn check_private_api_impl<'v>(
    allowlist: Value<'v>,
    depth: i32,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<()> {
    // This method may be called anywhere from builtins, but not outside (because it's not
    // exposed in cc_common.bzl
    // Java: net/starlark/java/eval/Module.java:121 Module.ofInnermostEnclosingStarlarkFunction
    let depth = depth.max(0) as usize;
    if eval.innermost_enclosing_def_module(depth).is_none() {
        // The module is null when the call is coming from one of the callbacks passed to
        // execution phase
        return Ok(());
    }
    // Java: (BazelModuleContext) module.getClientData(): the client data of the function's own
    // module (not a lookup by file name); a null one is dereferenced by
    // BuiltinRestriction.failIfModuleOutsideAllowlist, after the allowlist is cast.
    let bazel_module_context =
        blaze_starlark::defining_module::module_extra_value_of_innermost_enclosing_starlark_function(
            eval, depth,
        )
        .map(|client_data| {
            blaze_packages::bzl_context::BazelModuleContext::of_module_extra_value(client_data)
                .expect("ClassCastException: the module's client data is not a BazelModuleContext")
        });
    let mut entries: Vec<(String, String)> = Vec::new();
    for item in sequence_elements(allowlist, eval.heap())? {
        // TODO(bazel-team): Avoid unchecked indexing and casts on values obtained from
        // Starlark, even though it is allowlisted.
        let tuple = TupleRef::from_value(item).ok_or_else(|| {
            errorf(format!(
                "at index 0 of allowlist, got element of type {}, want tuple",
                item.get_type()
            ))
        })?;
        let get = |i: usize| -> starlark::Result<String> {
            tuple
                .content()
                .get(i)
                .and_then(|v| v.unpack_str())
                .map(str::to_owned)
                .ok_or_else(|| errorf("ClassCastException in allowlist entry"))
        };
        entries.push((get(0)?, get(1)?));
    }
    // Java: BuiltinRestriction.failIfModuleOutsideAllowlist -> failIfLabelOutsideAllowlist.
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:190 BuiltinRestriction.failIfModuleOutsideAllowlist
    let bazel_module_context =
        bazel_module_context.expect("NullPointerException: no BazelModuleContext for the module");
    let label = &bazel_module_context.label;
    if label.get_repository() == &*blaze_cmdline::repository_name::BUILTINS {
        return Ok(());
    }
    if !entries.iter().any(|(repo, pkg)| {
        allowlist_entry_allows(repo, pkg, label, &bazel_module_context.repo_mapping)
    }) {
        return Err(errorf(format!(
            "file '{}' cannot use private API",
            label.get_canonical_form()
        )));
    }
    Ok(())
}

/// The Starlark-free `CcToolchainProvider` data `CppCompileActionBuilder` reads.
fn cc_toolchain_compile_data<'v>(
    toolchain: CcToolchainProvider<'v>,
    heap: Heap<'v>,
) -> CcToolchainCompileData {
    fn s<T>(r: starlark::Result<T>) -> Result<T, String> {
        r.map_err(|e| e.to_string())
    }
    CcToolchainCompileData {
        compiler_files: s(toolchain.get_compiler_files()),
        compiler_files_without_includes: s(toolchain.get_compiler_files_without_includes()),
        as_files: s(toolchain.get_as_files()),
        all_files: s(toolchain.get_all_files()),
        builtin_include_files: s(toolchain.get_builtin_include_files(heap)),
        built_in_include_directories: s(toolchain.get_built_in_include_directories(heap)),
        grep_includes: s(toolchain.get_grep_includes()),
    }
}

/// `createCoptsFilter(Starlark.isNullOrNone(coptsFilterObject) ? null : (String) ...)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:759 CcStarlarkInternal.createCoptsFilter
fn copts_filter_of(v: Value) -> starlark::Result<blaze_cc_features::cc_common::CoptsFilter> {
    let s = if v.is_none() {
        None
    } else {
        Some(v.unpack_str().ok_or_else(|| {
            errorf(format!(
                "parameter 'copts_filter' got value of type '{}', want 'string or NoneType'",
                v.get_type()
            ))
        })?)
    };
    body::create_copts_filter(s).map_err(eval_error)
}

/// `createCppCompileActionBuilder(...)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:893 CcStarlarkInternal.createCppCompileActionBuilder
#[allow(clippy::too_many_arguments)]
fn create_cpp_compile_action_builder<'v>(
    rule_context: &RuleContext,
    toolchain_type: &str,
    cc_compilation_context: CcCompilationContext,
    cc_toolchain: CcToolchainProvider<'v>,
    configuration: Arc<BuildConfigurationValue>,
    copts_filter: blaze_cc_features::cc_common::CoptsFilter,
    feature_configuration: &blaze_cc_features::FeatureConfigurationForStarlark,
    source_artifact: Artifact,
    additional_compilation_inputs: Value<'v>,
    additional_include_scanning_roots: Value<'v>,
    output_file: Value<'v>,
    dotd_file: Value<'v>,
    diagnostics_file: Value<'v>,
    gcno_file: Option<Value<'v>>,
    dwo_file: Option<Value<'v>>,
    lto_indexing_file: Option<Value<'v>>,
    use_pic: bool,
    needs_include_validation: bool,
    heap: Heap<'v>,
) -> starlark::Result<CppCompileActionBuilder> {
    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileActionBuilder.java:84 CppCompileActionBuilder.<init>
    let mut action_owner = None;
    // The execution platform of the owner (Java `owner.getExecutionPlatform()`), kept for
    // `CppCompileAction.getExecutionOs`.
    let mut execution_platform = None;
    if rule_context.use_auto_exec_groups() {
        let exec_group =
            blaze_cmdline::Label::parse_canonical_unchecked(toolchain_type).to_string();
        action_owner = rule_context.get_action_owner_for(&exec_group);
        if action_owner.is_some() {
            execution_platform = rule_context.get_execution_platform_for(&exec_group);
        }
    }
    let owner = match action_owner {
        Some(owner) => owner,
        None => {
            execution_platform = rule_context.get_execution_platform();
            rule_context.get_action_owner()
        }
    };
    let mut builder = CppCompileActionBuilder::new(
        owner,
        cc_toolchain_compile_data(cc_toolchain, heap),
        configuration,
    );
    builder.set_execution_platform(execution_platform);
    builder
        .set_source_file(source_artifact)
        .set_cc_compilation_context(cc_compilation_context)
        .set_copts_filter(copts_filter)
        .set_feature_configuration(feature_configuration.get_feature_configuration().clone())
        .add_execution_info(&target_execution_info(rule_context));
    if sequence_len(
        additional_compilation_inputs,
        "additional_compilation_inputs",
        heap,
    )? > 0
    {
        builder.add_mandatory_inputs(sequence_cast_artifacts(
            additional_compilation_inputs,
            "additional_compilation_inputs",
            heap,
        )?);
    }
    if sequence_len(
        additional_include_scanning_roots,
        "additional_include_scanning_roots",
        heap,
    )? > 0
    {
        builder.add_additional_include_scanning_roots(sequence_cast_artifacts(
            additional_include_scanning_roots,
            "additional_include_scanning_roots",
            heap,
        )?);
    }
    builder.set_gcno_file(gcno_file.and_then(null_if_none_artifact));
    builder.set_dwo_file(dwo_file.and_then(null_if_none_artifact));
    builder.set_lto_indexing_file(lto_indexing_file.and_then(null_if_none_artifact));
    builder.set_outputs(
        null_if_none_artifact(output_file),
        null_if_none_artifact(dotd_file),
        null_if_none_artifact(diagnostics_file),
    );
    builder.set_pic_mode(use_pic);
    builder.set_needs_include_validation(needs_include_validation);
    Ok(builder)
}

/// `Sequence.size()` of a `Sequence` parameter (Java rejects other types at the call).
fn sequence_len<'v>(v: Value<'v>, param: &str, heap: Heap<'v>) -> starlark::Result<usize> {
    if !is_sequence(v) {
        return Err(errorf(format!(
            "in call to create_cc_compile_action(), parameter '{param}' got value of type '{}', \
             want 'sequence'",
            v.get_type()
        )));
    }
    Ok(sequence_elements(v, heap)?.len())
}

/// The `HeaderInfo`s of a `Sequence.cast(x, HeaderInfo.class, what)`.
fn cast_header_infos<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Vec<Arc<HeaderInfoData>>> {
    if !is_sequence(x) {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    }
    let mut out = Vec::new();
    for (i, v) in sequence_elements(x, heap)?.into_iter().enumerate() {
        match header_info_of(v) {
            Some(h) => out.push(h),
            None => {
                return Err(errorf(format!(
                    "at index {i} of {what}, got element of type {}, want HeaderInfo",
                    v.get_type()
                )));
            }
        }
    }
    Ok(out)
}

/// `Sequence.cast(x, Artifact.class, what).getImmutableList()` of an optional parameter whose
/// default is `[]`.
fn artifacts_or_empty<'v>(
    x: Option<Value<'v>>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Vec<Artifact>> {
    match x {
        None => Ok(Vec::new()),
        Some(v) => sequence_cast_artifacts(v, what, heap),
    }
}

/// `SolibSymlinkAction.getDynamicLibrarySymlinkInternal(context, library, symlinkName)`: creates
/// the symlink artifact and registers the action.
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:228 SolibSymlinkAction.getDynamicLibrarySymlinkInternal
fn dynamic_library_symlink_internal(
    handle: RuleContextHandle,
    library: Artifact,
    symlink_name: &PathFragment,
    eval: &Evaluator,
) -> starlark::Result<Artifact> {
    let (symlink, action) = {
        let rc = handle.borrow();
        // Ignore libraries that are already represented by the symlinks.
        let root = rc.get_bin_directory();
        let symlink = rc.get_shareable_artifact(symlink_name, &root);
        let action = solib::get_dynamic_library_symlink_internal(
            rc.get_action_owner(),
            library,
            symlink.clone(),
        );
        (symlink, action)
    };
    register_action(eval, Arc::new(action))?;
    Ok(symlink)
}

/// Java `OS.name()`.
fn os_name(os: blaze_shell::util::os::OS) -> &'static str {
    use blaze_shell::util::os::OS;
    match os {
        OS::Darwin => "DARWIN",
        OS::Freebsd => "FREEBSD",
        OS::Openbsd => "OPENBSD",
        OS::Linux => "LINUX",
        OS::Windows => "WINDOWS",
        OS::Unknown => "UNKNOWN",
    }
}

/// Java `StarlarkList.immutableCopyOf(items)`.
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
fn immutable_list_copy<'v>(items: Vec<Value<'v>>, eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
    starlark::values::list::alloc_immutable_list(eval.heap(), &items)
}

#[starlark_module]
fn cc_internal_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:111 CcStarlarkInternal.checkPrivateApi
    fn check_private_api<'v>(
        this: Value<'v>,
        #[starlark(require = named)] allowlist: Value<'v>,
        #[starlark(require = named, default = 1)] depth: i32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        check_private_api_impl(allowlist, depth, eval)?;
        Ok(NoneType)
    }

    /// Wraps a dictionary of build variables into CcToolchainVariables.
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:140 CcStarlarkInternal.getCcToolchainVariables
    fn cc_toolchain_variables<'v>(
        this: Value<'v>,
        #[starlark(require = named)] vars: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<CcToolchainVariablesValue> {
        let _ = this;
        if DictRef::from_value(vars).is_none() {
            return Err(errorf(format!(
                "in call to cc_toolchain_variables(), parameter 'vars' got value of type '{}', \
                 want 'dict'",
                vars.get_type()
            )));
        }
        let build_variables = cast_vars(vars, "vars", heap)?;
        Ok(CcToolchainVariablesValue(body::cc_toolchain_variables(
            &build_variables,
        )))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:159 CcStarlarkInternal.combineCcToolchainVariables
    fn combine_cc_toolchain_variables<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] parent: Value<'v>,
        #[starlark(args)] variables: UnpackTuple<Value<'v>>,
    ) -> starlark::Result<CcToolchainVariablesValue> {
        let _ = this;
        let parent = variables_param(parent, "combine_cc_toolchain_variables", "parent")?;
        // Java: Sequence.cast(variablesSequenceUnchecked, CcToolchainVariables.class, "variables")
        let mut variables_sequence: Vec<CcToolchainVariablesRef> = Vec::new();
        for (i, v) in variables.items.iter().enumerate() {
            match v.downcast_ref::<CcToolchainVariablesValue>() {
                Some(x) => variables_sequence.push(x.0.clone()),
                None => {
                    return Err(errorf(format!(
                        "at index {i} of variables, got element of type {}, want Variables",
                        v.get_type()
                    )));
                }
            }
        }
        Ok(CcToolchainVariablesValue(
            body::combine_cc_toolchain_variables(&parent, &variables_sequence),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:176 CcStarlarkInternal.internStringSequenceVariableValue
    fn intern_string_sequence_variable_value<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] string_sequence: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        // PORT: interning is a memory optimization; the strings are returned as a new
        // immutable list (`StarlarkList.immutableCopyOf`).
        let strings = sequence_cast_strings(string_sequence, "string_sequence", eval.heap())?;
        let items: Vec<Value<'v>> = strings
            .iter()
            .map(|s| eval.heap().alloc_str(s).to_value())
            .collect();
        Ok(immutable_list_copy(items, eval))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:195 CcStarlarkInternal.solibSymlinkAction
    fn solib_symlink_action<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] artifact: Value<'v>,
        #[starlark(require = named)] solib_directory: &str,
        #[starlark(require = named)] runtime_solib_dir_base: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        let handle = ctx_rule_context(ctx, "solib_symlink_action", "ctx")?;
        let artifact = artifact_param(artifact, "solib_symlink_action", "artifact")?;
        // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:209 SolibSymlinkAction.getCppRuntimeSymlink
        let symlink_name = solib::get_cpp_runtime_symlink_path(
            &artifact,
            solib_directory,
            Some(runtime_solib_dir_base),
        );
        Ok(StarlarkFile::new(dynamic_library_symlink_internal(
            handle,
            artifact,
            &symlink_name,
            eval,
        )?))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:217 CcStarlarkInternal.dynamicLibrarySymlinkAction
    fn dynamic_library_symlink<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        #[starlark(require = pos)] library: Value<'v>,
        #[starlark(require = pos)] solib_directory: &str,
        #[starlark(require = pos)] preserve_name: bool,
        #[starlark(require = pos)] prefix_consumer: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        let handle = actions_rule_context(actions, "dynamic_library_symlink", "actions")?;
        let library = artifact_param(library, "dynamic_library_symlink", "library")?;
        // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:158 SolibSymlinkAction.getDynamicLibrarySymlink
        let mangled_name = {
            let rc = handle.borrow();
            solib::get_dynamic_library_symlink_path(
                rc.get_label(),
                solib_directory,
                rc.get_configuration().get_mnemonic(),
                &library,
                preserve_name,
                prefix_consumer,
            )
        };
        Ok(StarlarkFile::new(dynamic_library_symlink_internal(
            handle,
            library,
            &mangled_name,
            eval,
        )?))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:236 CcStarlarkInternal.dynamicLibrarySymlinkAction2
    fn dynamic_library_symlink2<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        #[starlark(require = pos)] library: Value<'v>,
        #[starlark(require = pos)] solib_directory: &str,
        #[starlark(require = pos)] path: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        let handle = actions_rule_context(actions, "dynamic_library_symlink2", "actions")?;
        let library = artifact_param(library, "dynamic_library_symlink2", "library")?;
        // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:189 SolibSymlinkAction.getDynamicLibrarySymlink
        let link_name = solib::get_dynamic_library_symlink_path_for_path(
            solib_directory,
            &library,
            &PathFragment::create(path),
        );
        Ok(StarlarkFile::new(dynamic_library_symlink_internal(
            handle, library, &link_name, eval,
        )?))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:250 CcStarlarkInternal.dynamicLibrarySoname
    fn dynamic_library_soname<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = pos)] preserve_name: bool,
    ) -> starlark::Result<String> {
        let _ = this;
        // Java declares `WrappedStarlarkActionFactory actions`.
        let Some(wrapped) = actions.downcast_ref::<WrappedStarlarkActionFactory>() else {
            return Err(errorf(format!(
                "in call to dynamic_library_soname(), parameter 'actions' got value of type \
                 '{}', want 'WrappedStarlarkActionFactory'",
                actions.get_type()
            )));
        };
        // Java: actions.construction.getContext().getConfiguration().getMnemonic()
        let mnemonic = wrapped
            .construction()
            .get_context()
            .borrow()
            .get_configuration()
            .get_mnemonic()
            .to_owned();
        Ok(solib::get_dynamic_library_soname(
            &PathFragment::create(path),
            preserve_name,
            &mnemonic,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:263 CcStarlarkInternal.ccToolchainFeatures
    fn cc_toolchain_features<'v>(
        this: Value<'v>,
        #[starlark(require = named)] toolchain_config_info: Value<'v>,
        #[starlark(require = named)] tools_directory: &str,
    ) -> starlark::Result<CcToolchainFeaturesValue> {
        let _ = this;
        let Some(info) = blaze_starlark::structs::StarlarkInfo::from_value(toolchain_config_info)
        else {
            return Err(errorf(format!(
                "in call to cc_toolchain_features(), parameter 'toolchain_config_info' got value \
                 of type '{}', want 'struct'",
                toolchain_config_info.get_type()
            )));
        };
        // Java: `CcToolchainConfigInfo.PROVIDER.wrap(ccToolchainConfigInfo)` is
        // `new CcToolchainConfigInfo((StarlarkInfo) value)`: the provider of the value is not
        // checked (rules/cpp/CcToolchainConfigInfo.java:106); a value without the fields fails
        // when `CcToolchainFeatures` reads them.
        let converted = to_cc_starlark_info(info);
        let features = body::cc_toolchain_features(Arc::new(converted), tools_directory)
            .map_err(eval_error)?;
        Ok(CcToolchainFeaturesValue(Arc::new(features)))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:275 CcStarlarkInternal.isPackageHeadersCheckingModeSetForStarlark
    fn is_package_headers_checking_mode_set<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = this;
        let handle = ctx_rule_context(ctx, "is_package_headers_checking_mode_set", "ctx")?;
        let rc = handle.borrow();
        Ok(rc
            .get_rule()
            .get_package_declarations()
            .get_package_args()
            .is_default_hdrs_check_set())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:289 CcStarlarkInternal.getPackageHeadersCheckingModeForStarlark
    fn package_headers_checking_mode<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = this;
        let handle = ctx_rule_context(ctx, "package_headers_checking_mode", "ctx")?;
        let rc = handle.borrow();
        Ok(rc
            .get_rule()
            .get_package_declarations()
            .get_package_args()
            .get_default_hdrs_check()
            .to_owned())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:305 CcStarlarkInternal.isPackageHeadersCheckingModeSetForStarlarkAspect
    fn is_package_headers_checking_mode_set_for_aspect<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = (this, ctx);
        // Java reads `ruleContext.getTarget().getPackageDeclarations()` (the aspect's target).
        blaze_util::unported!("CcStarlarkInternal.isPackageHeadersCheckingModeSetForStarlarkAspect")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:318 CcStarlarkInternal.getPackageHeadersCheckingModeForStarlarkAspect
    fn package_headers_checking_mode_for_aspect<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = (this, ctx);
        blaze_util::unported!("CcStarlarkInternal.getPackageHeadersCheckingModeForStarlarkAspect")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:343 CcStarlarkInternal.getStlComputedDefault
    fn stl_computed_default<'v>(this: Value<'v>) -> starlark::Result<NoneType> {
        let _ = this;
        // Needs a native ComputedDefault attribute value (blaze_packages::attribute).
        blaze_util::unported!("CcStarlarkInternal.getStlComputedDefault")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:353 CcStarlarkInternal.getArtifactNameForCategory
    fn get_artifact_name_for_category<'v>(
        this: Value<'v>,
        #[starlark(require = named)] cc_toolchain: Value<'v>,
        #[starlark(require = named)] category: &str,
        #[starlark(require = named)] output_name: &str,
    ) -> starlark::Result<String> {
        let _ = this;
        let cc_toolchain = CcToolchainProvider::wrap(cc_toolchain)?;
        let Some(features) = cc_toolchain.get_features()? else {
            panic!("NullPointerException: CcToolchainProvider.getFeatures");
        };
        Ok(body::get_artifact_name_for_category(
            &features,
            category,
            output_name,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:368 CcStarlarkInternal.getArtifactNameExtensionForCategory
    fn get_artifact_name_extension_for_category<'v>(
        this: Value<'v>,
        cc_toolchain: Value<'v>,
        category: &str,
    ) -> starlark::Result<String> {
        let _ = this;
        let cc_toolchain = CcToolchainProvider::wrap(cc_toolchain)?;
        let Some(features) = cc_toolchain.get_features()? else {
            panic!("NullPointerException: CcToolchainProvider.getFeatures");
        };
        Ok(body::get_artifact_name_extension_for_category(
            &features, category,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:388 CcStarlarkInternal.absoluteSymlink
    fn absolute_symlink<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] output: Value<'v>,
        #[starlark(require = named)] target_path: &str,
        #[starlark(require = named)] progress_message: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        // Java: the parameter is typed `StarlarkActionContext` (a rule or a subrule context);
        // `Starlark.classType` of that interface is its simple name.
        let Some(handle) =
            blaze_analysis::starlark::starlark_subrule::starlark_action_context_rule_context(ctx)
        else {
            return Err(errorf(format!(
                "in call to absolute_symlink(), parameter 'ctx' got value of type '{}', want \
                 'StarlarkActionContext'",
                ctx.get_type()
            )));
        };
        let output = artifact_param(output, "absolute_symlink", "output")?;
        let action = SymlinkAction::to_absolute_path(
            handle.borrow().get_action_owner(),
            PathFragment::create(target_path),
            output,
            Some(progress_message.to_owned()),
        );
        register_action(eval, Arc::new(action))?;
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:403 CcStarlarkInternal.internList
    fn intern_seq<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] seq: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        if !is_sequence(seq) {
            return Err(errorf(format!(
                "in call to intern_seq(), parameter 'seq' got value of type '{}', want \
                 'sequence'",
                seq.get_type()
            )));
        }
        // PORT: interning is a memory optimization; the elements are copied into a tuple.
        let items = sequence_elements(seq, heap)?;
        Ok(heap.alloc(AllocTuple(items)))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:420 CcStarlarkInternal.getArgs
    fn get_link_args<'v>(
        this: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] build_variables: Value<'v>,
        #[starlark(require = named)] parameter_file_type: Value<'v>,
    ) -> starlark::Result<RegisteredArgs> {
        let _ = this;
        let feature_configuration = feature_configuration_param(
            feature_configuration,
            "get_link_args",
            "feature_configuration",
        )?;
        let build_variables = variables_param(build_variables, "get_link_args", "build_variables")?;
        let param_file_type = if parameter_file_type.is_none() {
            None
        } else {
            Some(parameter_file_type.unpack_str().ok_or_else(|| {
                errorf(format!(
                    "in call to get_link_args(), parameter 'parameter_file_type' got value of \
                     type '{}', want 'string or NoneType'",
                    parameter_file_type.get_type()
                ))
            })?)
        };
        let (link_command_line, param_file_info) = body::get_link_args(
            action_name,
            &feature_configuration,
            build_variables,
            param_file_type,
        )
        .map_err(eval_error)?;
        Ok(RegisteredArgs::for_registered_action(
            Arc::new(LinkCommandLineAdapter(link_command_line)),
            param_file_info.as_ref().map(to_actions_param_file_info),
            Vec::new(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:473 CcStarlarkInternal.wrapLinkActions
    fn wrap_link_actions<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        #[starlark(require = pos, default = NoneType)] build_configuration: Value<'v>,
        #[starlark(require = pos, default = false)] sharable_artifacts: bool,
    ) -> starlark::Result<WrappedStarlarkActionFactory<'v>> {
        let _ = this;
        let handle = actions_rule_context(actions, "wrap_link_actions", "actions")?;
        let config = match build_configuration
            .downcast_ref::<blaze_analysis::starlark::starlark_configuration::StarlarkConfiguration>()
        {
            Some(c) => c.0.clone(),
            None => handle.borrow().get_configuration().clone(),
        };
        let construction = new_action_construction(handle, config, sharable_artifacts);
        // Java: new WrappedStarlarkActionFactory(actions, construction) copies the parent; a
        // wrapped factory passed in wraps its own parent.
        // PORT: the copy of a wrapped factory is a plain StarlarkActionFactory over the same
        // rule context, i.e. the wrapped factory's parent.
        let parent = match actions.downcast_ref::<WrappedStarlarkActionFactory>() {
            Some(w) => w.parent(),
            None => actions,
        };
        Ok(WrappedStarlarkActionFactory::new(parent, construction))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:491 CcStarlarkInternal.getStarlarkRuleContext
    fn actions2ctx_cheat<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] actions: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let _ = actions_rule_context(actions, "actions2ctx_cheat", "actions")?;
        // Java: actions.getRuleContext().getStarlarkRuleContext(): the ctx of the running rule.
        blaze_analysis::starlark::thread_payload::get_starlark_rule_context(eval)
            .ok_or_else(|| errorf("actions2ctx_cheat: no rule context is being evaluated"))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:500 CcStarlarkInternal.getExecOs
    fn exec_os<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = this;
        let handle = ctx_rule_context(ctx, "exec_os", "ctx")?;
        let rc = handle.borrow();
        let platform = rc.get_execution_platform();
        Ok(
            os_name(blaze_analysis::platform::get_os_from_constraints_or_host(
                platform.as_deref(),
            ))
            .to_owned(),
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:508 CcStarlarkInternal.getRuleClass
    fn rule_class<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = this;
        let handle = ctx_rule_context(ctx, "rule_class", "ctx")?;
        let rc = handle.borrow();
        Ok(rc.get_rule().get_rule_class().to_owned())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:516 CcStarlarkInternal.getAspectClass
    fn aspect_class<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] ctx: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let Some(c) = starlark_rule_context_from_value(ctx) else {
            return Err(errorf(format!(
                "in call to aspect_class(), parameter 'ctx' got value of type '{}', want 'ctx'",
                ctx.get_type()
            )));
        };
        if !c.is_for_aspect() {
            return Ok(Value::new_none());
        }
        blaze_util::unported!("CcStarlarkInternal.getAspectClass (aspect descriptor)")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:532 CcStarlarkInternal.collectPerFileLtoBackendOpts
    fn collect_per_file_lto_backend_opts<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] cpp_config: Value<'v>,
        #[starlark(require = pos)] obj: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let cpp_configuration = cpp_configuration_param(
            cpp_config,
            "collect_per_file_lto_backend_opts",
            "cpp_config",
        )?;
        let object_file = artifact_param(obj, "collect_per_file_lto_backend_opts", "obj")?;
        let mut options = Vec::new();
        for per_label_options in cpp_configuration.get_per_file_lto_backend_opts() {
            // Java: PerLabelOptions.isIncluded(Artifact) uses getExecPathString().
            if per_label_options.is_included_exec_path(object_file.get_exec_path_string()) {
                options.extend(per_label_options.get_options().iter().cloned());
            }
        }
        Ok(mutable_string_list(heap, &options))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:552 CcStarlarkInternal.isTreeArtifact
    fn is_tree_artifact<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] artifact: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = this;
        let artifact = artifact_param(artifact, "is_tree_artifact", "artifact")?;
        Ok(artifact.is_tree_artifact())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:563 CcStarlarkInternal.computeOutputNamePrefixDir
    fn compute_output_name_prefix_dir<'v>(
        this: Value<'v>,
        #[starlark(require = named)] configuration: Value<'v>,
        #[starlark(require = named)] purpose: &str,
    ) -> starlark::Result<String> {
        let _ = this;
        let configuration = configuration_param(
            configuration,
            "compute_output_name_prefix_dir",
            "configuration",
        )?;
        let mut output_name_prefix_dir: Option<&str> = None;
        // purpose is only used by objc rules; if set it ends with either "_non_objc_arc" or
        // "_objc_arc", and it is used to override configuration.getMnemonic() to prefix the
        // output dir with "non_arc" or "arc".
        // PORT: Java's `purpose != null` is always true (the parameter is a String).
        let mnemonic = purpose;
        let _ = configuration.get_mnemonic();
        if mnemonic.ends_with("_objc_arc") {
            output_name_prefix_dir = Some(if mnemonic.ends_with("_non_objc_arc") {
                "non_arc"
            } else {
                "arc"
            });
        }
        Ok(output_name_prefix_dir.unwrap_or("").to_owned())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:665 CcStarlarkInternal.createCppCompileAction
    fn create_cc_compile_action<'v>(
        this: Value<'v>,
        #[starlark(require = named)] action_construction_context: Value<'v>,
        #[starlark(require = named)] cc_compilation_context: Value<'v>,
        #[starlark(require = named)] cc_toolchain: Value<'v>,
        #[starlark(require = named)] configuration: Value<'v>,
        #[starlark(require = named, default = NoneType)] copts_filter: Value<'v>,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] source: Value<'v>,
        #[starlark(require = named)] additional_compilation_inputs: Option<Value<'v>>,
        #[starlark(require = named, default = NoneType)] additional_compilation_inputs_set: Value<
            'v,
        >,
        #[starlark(require = named)] additional_include_scanning_roots: Option<Value<'v>>,
        #[starlark(require = named, default = NoneType)] output_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] dotd_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] diagnostics_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] gcno_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] dwo_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] lto_indexing_file: Value<'v>,
        #[starlark(require = named, default = false)] use_pic: bool,
        #[starlark(require = named)] compile_build_variables: Value<'v>,
        #[starlark(require = named, default = NoneType)] cache_key_inputs: Value<'v>,
        #[starlark(require = named, default = NoneType)] build_info_header_files: Value<'v>,
        #[starlark(require = named, default = NoneType)] action_name: Value<'v>,
        #[starlark(require = named, default = NoneType)] should_scan_includes: Value<'v>,
        #[starlark(require = named, default = NoneType)] shareable: Value<'v>,
        #[starlark(require = named, default = NoneType)] module_files: Value<'v>,
        #[starlark(require = named, default = NoneType)] modmap_file: Value<'v>,
        #[starlark(require = named, default = NoneType)] modmap_input_file: Value<'v>,
        #[starlark(require = named)] additional_outputs: Option<Value<'v>>,
        #[starlark(require = named, default = false)] needs_include_validation: bool,
        #[starlark(require = named)] toolchain_type: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        let heap = eval.heap();
        const F: &str = "create_cc_compile_action";
        let handle = ctx_rule_context(
            action_construction_context,
            F,
            "action_construction_context",
        )?;
        let feature_configuration =
            feature_configuration_param(feature_configuration, F, "feature_configuration")?;
        let compile_build_variables =
            variables_param(compile_build_variables, F, "compile_build_variables")?;
        let configuration = configuration_param(configuration, F, "configuration")?;
        let source_artifact = artifact_param(source, F, "source")?;
        let cc_toolchain = CcToolchainProvider::of_param(cc_toolchain, F, "cc_toolchain")?;
        let empty_list = heap.alloc(AllocList(Vec::<Value<'v>>::new()));
        let additional_compilation_inputs = additional_compilation_inputs.unwrap_or(empty_list);
        let additional_include_scanning_roots =
            additional_include_scanning_roots.unwrap_or(empty_list);

        let copts_filter = copts_filter_of(copts_filter)?;
        let mut builder = {
            let rc = handle.borrow();
            create_cpp_compile_action_builder(
                &rc,
                toolchain_type,
                CcCompilationContext::of_value(cc_compilation_context)?,
                cc_toolchain,
                configuration,
                copts_filter,
                &feature_configuration,
                source_artifact,
                additional_compilation_inputs,
                additional_include_scanning_roots,
                output_file,
                dotd_file,
                diagnostics_file,
                Some(gcno_file),
                Some(dwo_file),
                Some(lto_indexing_file),
                use_pic,
                needs_include_validation,
                heap,
            )?
        };
        if blaze_starlark::depset::Depset::from_value(additional_compilation_inputs_set).is_some() {
            builder.add_mandatory_inputs_set(&depset_cast_artifacts(
                additional_compilation_inputs_set,
                "additional_compilation_inputs_set",
            )?);
        }
        builder.set_variables(compile_build_variables);
        if !cache_key_inputs.is_none() {
            builder
                .set_cache_key_inputs(depset_cast_artifacts(cache_key_inputs, "cache_key_inputs")?);
        }
        if !build_info_header_files.is_none() {
            builder.set_build_info_header_artifacts(sequence_cast_artifacts(
                build_info_header_files,
                "builtin_header_files",
                heap,
            )?);
        }
        if let Some(action_name) = action_name.unpack_str() {
            builder.set_action_name(action_name);
        }
        if let Some(b) = should_scan_includes.unpack_bool() {
            builder.set_should_scan_includes(b);
        }
        if let Some(b) = shareable.unpack_bool() {
            builder.set_shareable(b);
        }
        builder.set_module_files(depset_noneable_cast_artifacts(
            module_files,
            "module_files",
        )?);
        builder.set_modmap_file(null_if_none_artifact(modmap_file));
        builder.set_modmap_input_file(null_if_none_artifact(modmap_input_file));
        builder.set_additional_outputs(artifacts_or_empty(
            additional_outputs,
            "additional_outputs",
            heap,
        )?);
        match builder.build_and_verify() {
            Ok(compile_action) => {
                register_action(eval, Arc::new(compile_action))?;
            }
            Err(BuildAndVerifyError::UnconfiguredActionConfig(_)) => {
                return Err(errorf(format!(
                    "Expected action_config for '{}' to be configured",
                    builder.get_action_name()
                )));
            }
            Err(BuildAndVerifyError::Eval(e)) => return Err(e),
        }
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:811 CcStarlarkInternal.createCppCompileActionTemplate
    fn create_cc_compile_action_template<'v>(
        this: Value<'v>,
        #[starlark(require = named)] action_construction_context: Value<'v>,
        #[starlark(require = named)] cc_compilation_context: Value<'v>,
        #[starlark(require = named)] cc_toolchain: Value<'v>,
        #[starlark(require = named)] configuration: Value<'v>,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] compile_build_variables: Value<'v>,
        #[starlark(require = named)] source: Value<'v>,
        #[starlark(require = named)] additional_compilation_inputs: Option<Value<'v>>,
        #[starlark(require = named)] additional_include_scanning_roots: Option<Value<'v>>,
        #[starlark(require = named, default = false)] use_pic: bool,
        #[starlark(require = named)] output_categories: Value<'v>,
        #[starlark(require = named)] output_files: Value<'v>,
        #[starlark(require = named)] dotd_tree_artifact: Value<'v>,
        #[starlark(require = named)] diagnostics_tree_artifact: Value<'v>,
        #[starlark(require = named)] lto_indexing_tree_artifact: Value<'v>,
        #[starlark(require = named, default = NoneType)] copts_filter: Value<'v>,
        #[starlark(require = named)] needs_include_validation: bool,
        #[starlark(require = named)] toolchain_type: &str,
    ) -> starlark::Result<NoneType> {
        let _ = (
            this,
            action_construction_context,
            cc_compilation_context,
            cc_toolchain,
            configuration,
            feature_configuration,
            compile_build_variables,
            source,
            additional_compilation_inputs,
            additional_include_scanning_roots,
            use_pic,
            output_categories,
            output_files,
            dotd_tree_artifact,
            diagnostics_tree_artifact,
            lto_indexing_tree_artifact,
            copts_filter,
            needs_include_validation,
            toolchain_type,
        );
        // Needs CppCompileActionTemplate (an ActionTemplate over tree artifacts); not reached by
        // brotli or re2.
        blaze_util::unported!("CcStarlarkInternal.createCppCompileActionTemplate")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:954 CcStarlarkInternal.freeze
    fn freeze<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] value: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let heap = eval.heap();
        if DictRef::from_value(value).is_some() {
            // Java: Dict.immutableCopyOf(dict): an immutable dict of the same entries (the dict
            // itself when it is already immutable); keys and values are not copied.
            return blaze_starlark::dict::dict_immutable_copy_of(heap, value);
        }
        // Java `Iterable`: StarlarkList, Tuple, RangeList, StarlarkSet ->
        // StarlarkList.immutableCopyOf(iterable).
        if ListRef::from_value(value).is_some()
            || TupleRef::from_value(value).is_some()
            || matches!(value.get_type(), "range" | "set")
        {
            return blaze_starlark::list::list_immutable_copy_of(heap, value);
        }
        Ok(value)
    }

    // PORT: Java's positional-only parameter is named `fn`, a Rust keyword.
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:966 CcStarlarkInternal.checkToplevel
    fn check_toplevel<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] func: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        // Java: the parameter is typed `StarlarkFunction` (a `def` or `lambda`).
        if blaze_starlark::defining_module::module_file_of_function(func).is_none() {
            return Err(errorf(format!(
                "in call to check_toplevel(), parameter 'fn' got value of type '{}', want \
                 'function'",
                func.get_type()
            )));
        }
        // Java: fn.getModule().getGlobal(fn.getName()) != fn
        if !blaze_starlark::defining_module::is_global_of_defining_module(func, eval) {
            return Err(errorf("Passed function must be top-level functions."));
        }
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:978 CcStarlarkInternal.perFileCopts
    fn per_file_copts<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] cpp_configuration: Value<'v>,
        #[starlark(require = pos)] source_file: Value<'v>,
        #[starlark(require = pos)] label: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let cpp_configuration =
            cpp_configuration_param(cpp_configuration, "per_file_copts", "cpp_configuration")?;
        let source_file = artifact_param(source_file, "per_file_copts", "source_file")?;
        let Some(source_label) = label.downcast_ref::<LabelValue>() else {
            return Err(errorf(format!(
                "in call to per_file_copts(), parameter 'label' got value of type '{}', want \
                 'Label'",
                label.get_type()
            )));
        };
        let mut options = Vec::new();
        for per_label_options in cpp_configuration.get_per_file_copts() {
            if per_label_options.is_included(source_label.label())
                || per_label_options.is_included_exec_path(source_file.get_exec_path_string())
            {
                options.extend(per_label_options.get_options().iter().cloned());
            }
        }
        Ok(mutable_string_list(heap, &options))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1000 CcStarlarkInternal.declareCompileOutputFile
    fn declare_compile_output_file<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] label: Value<'v>,
        #[starlark(require = named)] output_name: &str,
        #[starlark(require = named)] configuration: Value<'v>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        const F: &str = "declare_compile_output_file";
        let handle = ctx_rule_context(ctx, F, "ctx")?;
        let Some(label) = label.downcast_ref::<LabelValue>() else {
            return Err(errorf(format!(
                "in call to {F}(), parameter 'label' got value of type '{}', want 'Label'",
                label.get_type()
            )));
        };
        let config = configuration_param(configuration, F, "configuration")?;
        // Java: com/google/devtools/build/lib/rules/cpp/CppHelper.java:85 CppHelper.getCompileOutputArtifact
        let label = label.label();
        // Java: com/google/devtools/build/lib/rules/cpp/CppHelper.java:69 CppHelper.getObjDirectory
        let object_dir = get_unique_directory(
            label,
            &PathFragment::create("_objs"),
            config.sibling_repository_layout(),
        );
        let rc = handle.borrow();
        Ok(StarlarkFile::new(rc.get_derived_artifact(
            &object_dir.get_relative(&PathFragment::create(output_name)),
            &to_actions_root(&config.get_bin_directory(label.get_repository())),
        )))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1018 CcStarlarkInternal.declareOtherOutputFile
    fn declare_other_output_file<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] output_name: &str,
        #[starlark(require = named)] object_file: Value<'v>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        const F: &str = "declare_other_output_file";
        let handle = ctx_rule_context(ctx, F, "ctx")?;
        let object_file = artifact_param(object_file, F, "object_file")?;
        let rc = handle.borrow();
        let path = object_file
            .get_root_relative_path()
            .get_parent_directory()
            .expect("NullPointerException: object file has no parent directory")
            .get_relative(&PathFragment::create(output_name));
        let root = to_actions_root(
            &rc.get_configuration()
                .get_bin_directory(rc.get_label().get_repository()),
        );
        Ok(StarlarkFile::new(rc.get_derived_artifact(&path, &root)))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1051 CcStarlarkInternal.createLtoBackendAction
    fn create_lto_backend_action<'v>(
        this: Value<'v>,
        #[starlark(require = named)] actions: Value<'v>,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] build_variables: Value<'v>,
        #[starlark(require = named)] use_pic: bool,
        #[starlark(require = named)] inputs: Value<'v>,
        #[starlark(require = named)] all_bitcode_files: Value<'v>,
        #[starlark(require = named)] imports: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        #[starlark(require = named)] env: Value<'v>,
    ) -> starlark::Result<NoneType> {
        let _ = (
            this,
            actions,
            feature_configuration,
            build_variables,
            use_pic,
            inputs,
            all_bitcode_files,
            imports,
            outputs,
            env,
        );
        // Needs LtoBackendAction / LtoBackendArtifacts (ThinLTO); not reached by brotli or re2.
        blaze_util::unported!("CcStarlarkInternal.createLtoBackendAction")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1130 CcStarlarkInternal.createLtoBackendActionTemplate
    fn create_lto_backend_action_template<'v>(
        this: Value<'v>,
        #[starlark(require = named)] actions: Value<'v>,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] build_variables: Value<'v>,
        #[starlark(require = named)] use_pic: bool,
        #[starlark(require = named)] all_bitcode_files: Value<'v>,
        #[starlark(require = named)] additional_inputs: Value<'v>,
        #[starlark(require = named)] index: Value<'v>,
        #[starlark(require = named)] bitcode_file: Value<'v>,
        #[starlark(require = named)] object_file: Value<'v>,
        #[starlark(require = named)] dwo_file: Value<'v>,
        #[starlark(require = named)] env: Value<'v>,
    ) -> starlark::Result<NoneType> {
        let _ = (
            this,
            actions,
            feature_configuration,
            build_variables,
            use_pic,
            all_bitcode_files,
            additional_inputs,
            index,
            bitcode_file,
            object_file,
            dwo_file,
            env,
        );
        // Needs LtoBackendActionTemplate; not reached by brotli or re2.
        blaze_util::unported!("CcStarlarkInternal.createLtoBackendActionTemplate")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1196 CcStarlarkInternal.createHeaderInfo
    fn create_header_info<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneType)] header_module: Value<'v>,
        #[starlark(require = named, default = NoneType)] pic_header_module: Value<'v>,
        #[starlark(require = named)] modular_public_headers: Option<Value<'v>>,
        #[starlark(require = named)] modular_private_headers: Option<Value<'v>>,
        #[starlark(require = named)] textual_headers: Option<Value<'v>>,
        #[starlark(require = named)] separate_module_headers: Option<Value<'v>>,
        #[starlark(require = named, default = NoneType)] separate_module: Value<'v>,
        #[starlark(require = named, default = NoneType)] separate_pic_module: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<HeaderInfo> {
        let _ = this;
        // Java: nullIfNone(x, Artifact.DerivedArtifact.class)
        let header_module = null_if_none_artifact(header_module);
        let pic_header_module = null_if_none_artifact(pic_header_module);
        let modular_public_headers =
            artifacts_or_empty(modular_public_headers, "modular_public_headers", heap)?;
        let modular_private_headers =
            artifacts_or_empty(modular_private_headers, "modular_private_headers", heap)?;
        let textual_headers = artifacts_or_empty(textual_headers, "textual_headers", heap)?;
        let separate_module_headers =
            artifacts_or_empty(separate_module_headers, "separate_module_headers", heap)?;
        Ok(HeaderInfo(HeaderInfoData::create(
            header_module,
            pic_header_module,
            &modular_public_headers,
            &modular_private_headers,
            &textual_headers,
            separate_module_headers,
            null_if_none_artifact(separate_module),
            null_if_none_artifact(separate_pic_module),
            Vec::new(),
            &[],
        )))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1234 CcStarlarkInternal.createHeaderInfoWithDeps
    fn create_header_info_with_deps<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneType)] header_info: Value<'v>,
        #[starlark(require = named)] deps: Option<Value<'v>>,
        #[starlark(require = named)] merged_deps: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<HeaderInfo> {
        let _ = this;
        let Some(header_info) = header_info_of(header_info) else {
            return Err(errorf(format!(
                "in call to create_header_info_with_deps(), parameter 'header_info' got value \
                 of type '{}', want 'HeaderInfo'",
                header_info.get_type()
            )));
        };
        let deps = match deps {
            Some(d) => cast_header_infos(d, "deps", heap)?,
            None => Vec::new(),
        };
        let merged_deps = match merged_deps {
            Some(d) => cast_header_infos(d, "merged_deps", heap)?,
            None => Vec::new(),
        };
        // Java passes the already-split header lists of `headerInfo` to `HeaderInfo.create` as
        // public/private/textual headers (so textual includes stay where they are).
        Ok(HeaderInfo(HeaderInfoData::create(
            header_info.header_module.clone(),
            header_info.pic_header_module.clone(),
            &header_info.modular_public_headers,
            &header_info.modular_private_headers,
            &header_info.textual_headers,
            header_info.separate_module_headers.clone(),
            header_info.separate_module.clone(),
            header_info.separate_pic_module.clone(),
            deps,
            &merged_deps,
        )))
    }

    /// Run variable expansion and shell tokenization on a sequence of flags.
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1270 CcStarlarkInternal.expandAndTokenize
    fn expand_and_tokenize<'v>(
        this: Value<'v>,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] attr: &str,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let flags = match flags {
            Some(f) => f,
            // Java: `defaultValue = "[]"`, the immutable empty list.
            None => return Ok(immutable_string_list(heap, &[])),
        };
        let flag_strings = sequence_cast_strings(flags, attr, heap)?;
        if flag_strings.is_empty() {
            // Java: `return Sequence.cast(flags, String.class, attributeName)`: `flags` itself.
            return Ok(flags);
        }
        let Some(starlark_ctx) = starlark_rule_context_from_value(ctx) else {
            return Err(errorf(format!(
                "in call to expand_and_tokenize(), parameter 'ctx' got value of type '{}', want \
                 'ctx'",
                ctx.get_type()
            )));
        };
        let handle = starlark_ctx.get_rule_context();
        let locations_prefers_executable =
            blaze_analysis::starlark::starlark_rule_context::get_starlark_semantics(starlark_ctx)
                .incompatible_locations_prefers_executable;
        let rc = handle.borrow();
        // Java: ImmutableSet.copyOf(Iterables.concat(getPrerequisites("srcs"), ...)).
        // PORT: configured targets are compared by identity in Java; (label, configuration) is
        // their identity within one build.
        let mut known = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for attribute in [
            "srcs",
            "non_arc_srcs",
            "hdrs",
            "data",
            "additional_linker_inputs",
        ] {
            for target in rc.get_prerequisites(attribute) {
                let key = (target.get_label(), target.get_configuration_key());
                if seen.insert(key) {
                    known.push(target);
                }
            }
        }
        let label_map = make_label_map(&known, locations_prefers_executable)?;
        let expander = rc
            .get_expander_with_label_map(label_map)
            .with_data_exec_locations();
        let expanded_flags = expander.tokenized(attr, &flag_strings);
        Ok(immutable_string_list(heap, &expanded_flags))
    }
}

/// `_builtins.internal.cc_internal`.
pub fn cc_internal() -> CcStarlarkInternal {
    CcStarlarkInternal
}
