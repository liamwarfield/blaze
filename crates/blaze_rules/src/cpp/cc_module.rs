//! `cc_common` as `_builtins.internal.cc_common` (Java `BazelCcModule` / `CcModule`, Starlark
//! API `CcModuleApi`): the natives `builtins_bzl/common/cc/cc_common_bazel.bzl` wraps.
//!
//! The Starlark-independent bodies are in `blaze_cc_features::cc_module`.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cc_features::cc_module as body;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::AllocDict;
use starlark::values::list::alloc_immutable_list;
use starlark::values::none::NoneType;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use super::cc_toolchain_provider::CcToolchainProvider;
use super::starlark_values::{
    CcToolchainVariablesValue, feature_configuration_param, variables_param,
};
use super::util::{errorf, is_called_from_starlark_cc_common};
use super::variables::{eval_error, expansion_error};

/// Java `BazelCcModule` (`cc_common`).
// Java: com/google/devtools/build/lib/bazel/rules/cpp/BazelCcModule.java:22 BazelCcModule
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:52 CcModule
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BazelCcModule;

starlark_simple_value!(BazelCcModule);

impl fmt::Display for BazelCcModule {
    // Java: StarlarkValue.repr default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<unknown object com.google.devtools.build.lib.bazel.rules.cpp.BazelCcModule>")
    }
}

#[starlark_value(type = "cc_common")]
impl<'v> StarlarkValue<'v> for BazelCcModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("cc_common", cc_module_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:62 CcModuleApi.getCcToolchainProvider
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        if attribute == "CcToolchainInfo" {
            // Java returns `CcToolchainProvider.BAZEL_PROVIDER`, a StarlarkProviderWrapper keyed
            // by `@rules_cc+//cc/private/rules_impl:cc_toolchain_info.bzl`; Blaze has no native
            // value standing for a Starlark provider key.
            blaze_util::unported!("CcModule.getCcToolchainProvider");
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "CcToolchainInfo"
    }
}

/// `thread.getSemantics()`.
fn semantics(eval: &Evaluator) -> starlark::Result<Arc<StarlarkSemantics>> {
    ThreadContext::get(eval)
        .map(|cx| cx.semantics.clone())
        .map_err(|e| errorf(e.to_string()))
}

/// `StarlarkList.immutableCopyOf(items)`.
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
fn string_list<'v>(eval: &Evaluator<'v, '_, '_>, items: &[String]) -> Value<'v> {
    let heap = eval.heap();
    let values: Vec<Value<'v>> = items.iter().map(|s| heap.alloc_str(s).to_value()).collect();
    alloc_immutable_list(heap, &values)
}

#[starlark_module]
fn cc_module_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:72 CcModuleApi.compilerFlagExists
    #[starlark(attribute)]
    fn do_not_use_tools_cpp_compiler_present(this: &BazelCcModule) -> starlark::Result<NoneType> {
        let _ = this;
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:62 CcModule.getToolForAction
    fn get_tool_for_action<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "get_tool_for_action",
            "feature_configuration",
        )?;
        is_called_from_starlark_cc_common(eval)?;
        body::get_tool_for_action(&fc, action_name).map_err(eval_error)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:77 CcModule.getToolRequirementForAction
    fn get_tool_requirement_for_action<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "get_tool_requirement_for_action",
            "feature_configuration",
        )?;
        is_called_from_starlark_cc_common(eval)?;
        let requirements = body::get_tool_requirement_for_action(&fc, action_name);
        Ok(string_list(eval, &requirements))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:88 CcModule.getExecutionRequirements
    fn get_execution_requirements<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "get_execution_requirements",
            "feature_configuration",
        )?;
        is_called_from_starlark_cc_common(eval)?;
        let requirements = body::get_execution_requirements(&fc, action_name);
        Ok(string_list(eval, &requirements))
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:849 CcModuleApi.isEnabled
    fn is_enabled<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] feature_name: &str,
    ) -> starlark::Result<bool> {
        let _ = (this, feature_configuration, feature_name);
        // Java's default interface method (not overridden by CcModule / BazelCcModule).
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:99 CcModule.actionIsEnabled
    fn action_is_enabled<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "action_is_enabled",
            "feature_configuration",
        )?;
        is_called_from_starlark_cc_common(eval)?;
        Ok(body::action_is_enabled(&fc, action_name))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:109 CcModule.getCommandLine
    fn get_memory_inefficient_command_line<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        #[starlark(require = named)] variables: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "get_memory_inefficient_command_line",
            "feature_configuration",
        )?;
        let variables = variables_param(
            variables,
            "get_memory_inefficient_command_line",
            "variables",
        )?;
        is_called_from_starlark_cc_common(eval)?;
        let command_line =
            body::get_command_line(&fc, action_name, &variables).map_err(expansion_error)?;
        Ok(string_list(eval, &command_line))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:121 CcModule.getEnvironmentVariable
    fn get_environment_variables<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] feature_configuration: Value<'v>,
        #[starlark(require = named)] action_name: &str,
        #[starlark(require = named)] variables: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        let fc = feature_configuration_param(
            feature_configuration,
            "get_environment_variables",
            "feature_configuration",
        )?;
        let variables = variables_param(variables, "get_environment_variables", "variables")?;
        is_called_from_starlark_cc_common(eval)?;
        let env = body::get_environment_variable(&fc, action_name, &variables)
            .map_err(expansion_error)?;
        let heap = eval.heap();
        let entries: Vec<(Value<'v>, Value<'v>)> = env
            .iter()
            .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
            .collect();
        // Java: Dict.immutableCopyOf(...).
        blaze_starlark::dict::dict_immutable_copy_of(heap, heap.alloc(AllocDict(entries)))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:135 CcModule.getVariables
    fn empty_variables<'v>(
        this: &BazelCcModule,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<CcToolchainVariablesValue> {
        let _ = this;
        is_called_from_starlark_cc_common(eval)?;
        Ok(CcToolchainVariablesValue(body::get_variables()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:155 CcModule.checkExperimentalCcSharedLibrary
    fn check_experimental_cc_shared_library<'v>(
        this: &BazelCcModule,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        is_called_from_starlark_cc_common(eval)?;
        Ok(semantics(eval)?.experimental_cc_shared_library)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:161 CcModule.getIncompatibleDisableObjcLibraryTransition
    fn incompatible_disable_objc_library_transition<'v>(
        this: &BazelCcModule,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        is_called_from_starlark_cc_common(eval)?;
        Ok(semantics(eval)?.incompatible_disable_objc_library_transition)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:169 CcModule.addGoExecGroupsToBinaryRules
    fn add_go_exec_groups_to_binary_rules<'v>(
        this: &BazelCcModule,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        Ok(semantics(eval)?.add_go_exec_groups_to_binary_rules)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:175 CcModule.legacyCcFlagsMakeVariable
    fn legacy_cc_flags_make_variable_do_not_use<'v>(
        this: &BazelCcModule,
        #[starlark(require = named)] cc_toolchain: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        let _ = this;
        is_called_from_starlark_cc_common(eval)?;
        // Java: CcToolchainProvider.wrapOrThrowEvalException(ccToolchainInfo)
        let cc_toolchain = CcToolchainProvider::wrap(cc_toolchain)?;
        match cc_toolchain.get_legacy_cc_flags_make_variable()? {
            Some(s) => Ok(s),
            // Java returns null, which Starlark rejects for a String-returning method.
            None => panic!(
                "IllegalStateException: method legacy_cc_flags_make_variable_do_not_use returned \
                 null"
            ),
        }
    }
}

/// `_builtins.internal.cc_common`.
pub fn cc_common() -> BazelCcModule {
    BazelCcModule
}
