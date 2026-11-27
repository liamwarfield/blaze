//! The `testing` and `coverage_common` modules.
//!
//! Java: rules/test/StarlarkTestingModule.java (starlarkbuildapi/test/TestingModuleApi.java),
//! analysis/test/ExecutionInfo.java (`ExecutionInfo.PROVIDER`) and
//! analysis/test/CoverageCommon.java (starlarkbuildapi/test/CoverageCommonApi.java).
//! `testing.analysis_test` is ported (`rule_class_functions::analysis_test`); the other
//! functions build analysis-time objects and are not ported; the modules, their members and the
//! providers exist with Java's names, types and reprs.

use std::fmt;

use allocative::Allocative;
use blaze_starlark::Phase;
use blaze_starlark::ThreadContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::starlark_defs::builtin_providers::BuiltinConstructor;
use crate::starlark_defs::builtin_providers::BuiltinProvider;
use crate::starlark_defs::builtin_providers::BuiltinProviderData;
use crate::starlark_defs::builtin_providers::BuiltinProviderId;
use crate::starlark_defs::util::errorf;

// Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:74 ExecutionInfo.ExecutionInfoProvider.<init>
pub const EXECUTION_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "ExecutionInfo",
    value_class: "ExecutionInfo",
};

/// Java `ExecutionInfo.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:35 ExecutionInfo.PROVIDER
pub const fn execution_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: EXECUTION_INFO,
        // PORT: analysis-time (needs ExecutionInfo instances).
        constructor: BuiltinConstructor::Unported(
            "ExecutionInfo.ExecutionInfoProvider.constructor",
        ),
    })
}

// -------------------------------------------------------------------------------------------
// testing

/// The `testing` module (Java `StarlarkTestingModule`).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkTestingModule;

starlark_simple_value!(StarlarkTestingModule);

impl fmt::Display for StarlarkTestingModule {
    // Java: StarlarkValue.repr default (StarlarkTestingModule has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.rules.test.StarlarkTestingModule>"
        )
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingModuleApi.java:35 TestingModuleApi
#[starlark_value(type = "testing")]
impl<'v> StarlarkValue<'v> for StarlarkTestingModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("testing", testing_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:47 StarlarkTestingModule.executionInfo
            "ExecutionInfo" => Some(heap.alloc(execution_info())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "ExecutionInfo"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["ExecutionInfo".to_owned()]
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'testing'"))
    }
}

/// `StarlarkTestingModule.testEnvironment` as analysis implements it (the `RunEnvironmentInfo`
/// type is blaze_analysis's), installed with [`set_test_environment_hook`].
pub type TestEnvironmentHook =
    for<'v> fn(Value<'v>, Option<Value<'v>>, Heap<'v>) -> starlark::Result<Value<'v>>;

static TEST_ENVIRONMENT_HOOK: std::sync::OnceLock<TestEnvironmentHook> = std::sync::OnceLock::new();

/// Installs the constructor of `testing.TestEnvironment`'s `RunEnvironmentInfo` (first call wins).
pub fn set_test_environment_hook(hook: TestEnvironmentHook) {
    let _ = TEST_ENVIRONMENT_HOOK.set(hook);
}

#[allow(non_snake_case)]
#[starlark_module]
fn testing_methods(builder: &mut MethodsBuilder) {
    /// Java's one `testing` object serves both phases; a call while a .bzl file is loaded
    /// builds the same `RunEnvironmentInfo`, through the constructor blaze_analysis installs
    /// ([`set_test_environment_hook`]).
    // Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:52 StarlarkTestingModule.testEnvironment
    fn TestEnvironment<'v>(
        this: Value<'v>,
        environment: Value<'v>,
        inherited_environment: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        // The blaze binary's .bzl globals hold blaze_analysis' `testing` instead of this one
        // (blaze_rules `analysis_bzl_overrides`); this is reached from the loading-only setups
        // (dev tools, tests), where the hook may not be installed.
        let Some(hook) = TEST_ENVIRONMENT_HOOK.get() else {
            blaze_util::unported!(
                "StarlarkTestingModule.testEnvironment",
                "RunEnvironmentInfo without blaze_analysis::starlark::install_hooks"
            )
        };
        hook(environment, inherited_environment, heap)
    }

    // Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:62 StarlarkTestingModule.analysisTest
    fn analysis_test<'v>(
        this: Value<'v>,
        name: Value<'v>,
        implementation: Value<'v>,
        attrs: Option<Value<'v>>,
        fragments: Option<Value<'v>>,
        toolchains: Option<Value<'v>>,
        attr_values: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        // Java: the @Param types of TestingModuleApi.analysisTest, checked in order.
        let check = |param: &str, v: Option<Value<'v>>, ok: fn(Value<'v>) -> bool, want: &str| {
            match v {
                Some(v) if !ok(v) => Err(errorf(format!(
                    "in call to analysis_test(), parameter '{param}' got value of type '{}', want '{want}'",
                    v.get_type()
                ))),
                _ => Ok(()),
            }
        };
        check("name", Some(name), |v| v.unpack_str().is_some(), "string")?;
        check(
            "implementation",
            Some(implementation),
            |v| v.get_type() == "function",
            "function",
        )?;
        check("attrs", attrs, |v| v.get_type() == "dict", "dict")?;
        check("fragments", fragments, is_sequence, "sequence")?;
        check("toolchains", toolchains, is_sequence, "sequence")?;
        check(
            "attr_values",
            attr_values,
            |v| v.get_type() == "dict",
            "dict",
        )?;
        // TODO(b/236456122): Refactor this check into a standard helper / error message
        // PORT: Java checks for a Package.Builder and a RuleDefinitionEnvironment on the thread;
        // both exist exactly in BUILD (loading-phase) threads.
        let cx = ThreadContext::get(eval)?;
        if cx.phase != Phase::Loading
            || crate::package::target_definition_context::TargetDefinitionContext::from_or_null(
                eval,
            )
            .is_none()
        {
            return Err(errorf("analysis_test can only be called in a BUILD thread"));
        }
        let empty = || eval.heap().alloc(starlark::values::list::AllocList::EMPTY);
        let attrs =
            attrs.unwrap_or_else(|| eval.heap().alloc(starlark::values::dict::AllocDict::EMPTY));
        let fragments = fragments.unwrap_or_else(empty);
        let toolchains = toolchains.unwrap_or_else(empty);
        let attr_values = attr_values
            .unwrap_or_else(|| eval.heap().alloc(starlark::values::dict::AllocDict::EMPTY));
        crate::starlark_defs::rule_class_functions::analysis_test(
            eval,
            name.unpack_str().unwrap(),
            implementation,
            attrs,
            fragments,
            toolchains,
            attr_values,
        )?;
        Ok(NoneType)
    }
}

/// Java `instanceof Sequence` (list or tuple).
fn is_sequence(v: Value) -> bool {
    crate::starlark_defs::util::sequence_elements(v).is_some()
}

// -------------------------------------------------------------------------------------------
// coverage_common

/// The `coverage_common` module (Java `CoverageCommon`).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct CoverageCommon;

starlark_simple_value!(CoverageCommon);

impl fmt::Display for CoverageCommon {
    // Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:154 CoverageCommon.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<coverage_common>")
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/CoverageCommonApi.java:35 CoverageCommonApi
#[starlark_value(type = "coverage_common")]
impl<'v> StarlarkValue<'v> for CoverageCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("coverage_common", coverage_common_methods);
        Some(RES.methods())
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'coverage_common'"))
    }
}

#[starlark_module]
fn coverage_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/test/CoverageCommon.java:47 CoverageCommon.instrumentedFilesInfo
    fn instrumented_files_info<'v>(
        this: Value<'v>,
        ctx: Value<'v>,
        #[starlark(require = named)] source_attributes: Option<Value<'v>>,
        #[starlark(require = named)] dependency_attributes: Option<Value<'v>>,
        #[starlark(require = named)] coverage_support_files: Option<Value<'v>>,
        #[starlark(require = named)] coverage_environment: Option<Value<'v>>,
        #[starlark(require = named)] extensions: Option<Value<'v>>,
        #[starlark(require = named)] metadata_files: Option<Value<'v>>,
        #[starlark(require = named)] reported_to_actual_sources: Option<Value<'v>>,
        #[starlark(require = named)] baseline_coverage_files: Option<Value<'v>>,
    ) -> starlark::Result<NoneType> {
        let _ = (
            this,
            source_attributes,
            dependency_attributes,
            coverage_support_files,
            coverage_environment,
            extensions,
            metadata_files,
            reported_to_actual_sources,
            baseline_coverage_files,
        );
        // This is the loading-phase `coverage_common`: analysis registers its own (blaze_analysis
        // `register_coverage_common`). Java's `ctx` parameter has no allowedTypes, so the
        // parameter's class (`StarlarkRuleContext`, Starlark name "ctx") is the allowed type,
        // checked first; no value of that type exists while loading, so every call here fails
        // that check.
        // Java: net/starlark/java/eval/ParamDescriptor.java:92 ParamDescriptor.of
        Err(crate::starlark_defs::util::param_type_error(
            "instrumented_files_info",
            "ctx",
            ctx,
            "ctx",
        ))
    }
}

/// Registers `testing` and `coverage_common` (Java `TestingBootstrap`; its three providers are
/// in `builtin_providers::register_testing_providers`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingBootstrap.java:48 TestingBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_testing(builder: &mut GlobalsBuilder) {
    const testing: StarlarkTestingModule = StarlarkTestingModule;
    const coverage_common: CoverageCommon = CoverageCommon;
}

#[cfg(test)]
mod tests {
    use crate::starlark_defs::provider::test_support::error_message;
    use crate::starlark_defs::provider::test_support::exec_bzl;
    use crate::starlark_defs::provider::test_support::string_global;

    // Expected values recorded from the oracle (Bazel 9.3.2).

    #[test]
    fn testing_and_coverage_common_members() {
        let m = exec_bzl(
            "testing_test_1.bzl",
            r#"
def _p(x):
    return "%s|%s" % (type(x), repr(x))
result = ",".join([
    _p(testing), str(dir(testing)), _p(testing.ExecutionInfo),
    _p(coverage_common), str(dir(coverage_common)),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "testing|<unknown object com.google.devtools.build.lib.rules.test.StarlarkTestingModule>,\
             [\"ExecutionInfo\", \"TestEnvironment\", \"analysis_test\"],\
             Provider|<function ExecutionInfo>,\
             coverage_common|<coverage_common>,[\"instrumented_files_info\"]"
        );
    }

    #[test]
    fn analysis_test_outside_build_thread() {
        let e = error_message(exec_bzl(
            "testing_test_2.bzl",
            "testing.analysis_test(\"t\", lambda ctx: None)\n",
        ));
        assert!(
            e.contains("analysis_test can only be called in a BUILD thread"),
            "{e}"
        );
    }

    #[test]
    #[ignore = "needs RunEnvironmentInfo instances (analysis)"]
    fn test_environment() {}

    #[test]
    #[ignore = "needs BUILD loading (a package builder on the thread); covered end to end by \
                the oracle comparison of testing.analysis_test"]
    fn analysis_test() {}
}
