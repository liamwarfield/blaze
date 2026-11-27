//! Port of `rules/test/StarlarkTestingModule.java` (the `testing` module) with analysis-time
//! behaviour: `testing.ExecutionInfo` constructs `ExecutionInfo`s and `testing.TestEnvironment`
//! constructs `RunEnvironmentInfo`s.

use std::fmt;

use allocative::Allocative;
use blaze_starlark::{Phase, ThreadContext};
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::providers::run_environment_info::{
    RunEnvironmentInfo, dict_cast_string_string, sequence_cast_strings,
};
use crate::test::execution_info::{analysis_test_result_info_provider, execution_info_provider};

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The `testing` module (Java `StarlarkTestingModule`).
// Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:41 StarlarkTestingModule
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

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingModuleApi.java:36 TestingModuleApi
#[starlark_value(type = "testing")]
impl<'v> StarlarkValue<'v> for StarlarkTestingModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("testing", testing_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:45 StarlarkTestingModule.executionInfo
            "ExecutionInfo" => Some(heap.alloc(execution_info_provider())),
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
        Err(errorf("unhashable type: 'testing'".to_owned()))
    }
}

#[allow(non_snake_case)]
/// `testing.TestEnvironment(environment, inherited_environment)` called while a .bzl file is
/// loaded (the loading-phase `testing` module's hook, see
/// `blaze_packages::starlark_defs::testing::set_test_environment_hook`).
// Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:50 StarlarkTestingModule.testEnvironment
pub fn test_environment_at_load_time<'v>(
    environment: Value<'v>,
    inherited_environment: Option<Value<'v>>,
    heap: Heap<'v>,
) -> starlark::Result<Value<'v>> {
    let environment = dict_cast_string_string(environment, "environment")?;
    let inherited_environment = match inherited_environment {
        None => Vec::new(),
        Some(v) if v.is_none() => Vec::new(),
        Some(v) => sequence_cast_strings(v, "inherited_environment")?,
    };
    Ok(heap.alloc(RunEnvironmentInfo::new(
        environment,
        inherited_environment,
        /* should_error_on_non_executable_rule= */ false,
    )))
}

#[starlark_module]
fn testing_methods(builder: &mut MethodsBuilder) {
    /// Creates a new test environment provider (a `RunEnvironmentInfo`).
    // Java: com/google/devtools/build/lib/rules/test/StarlarkTestingModule.java:50 StarlarkTestingModule.testEnvironment
    fn TestEnvironment<'v>(
        this: Value<'v>,
        environment: Value<'v>,
        #[starlark(default = NoneOr::None)] inherited_environment: NoneOr<Value<'v>>,
    ) -> starlark::Result<RunEnvironmentInfo> {
        let _ = this;
        let environment = dict_cast_string_string(environment, "environment")?;
        let inherited_environment = match inherited_environment {
            NoneOr::None => Vec::new(),
            NoneOr::Other(v) => sequence_cast_strings(v, "inherited_environment")?,
        };
        Ok(RunEnvironmentInfo::new(
            environment,
            inherited_environment,
            /* should_error_on_non_executable_rule= */ false,
        ))
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
        // Java: the @Param types of TestingModuleApi.analysisTest, checked by the interpreter
        // before the method body, in parameter order.
        let check = |param: &str, v: Option<Value<'v>>, ok: fn(Value<'v>) -> bool, want: &str| {
            match v {
                Some(v) if !ok(v) => Err(errorf(format!(
                    "in call to analysis_test(), parameter '{param}' got value of type '{}', want '{want}'",
                    v.get_type()
                ))),
                _ => Ok(()),
            }
        };
        // Java's Sequence: StarlarkList, Tuple and RangeList.
        // Java: net/starlark/java/eval/RangeList.java:54 RangeList
        let is_sequence = |v: Value<'v>| {
            starlark::values::list::ListRef::from_value(v).is_some()
                || starlark::values::tuple::TupleRef::from_value(v).is_some()
                || v.get_type() == "range"
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
        // both exist exactly in BUILD (loading-phase) threads that hold a package builder.
        let cx = ThreadContext::get(eval)?;
        if cx.phase != Phase::Loading
            || blaze_packages::package::target_definition_context::TargetDefinitionContext::from_or_null(
                eval,
            )
            .is_none()
        {
            return Err(errorf(
                "analysis_test can only be called in a BUILD thread".to_owned(),
            ));
        }
        let Some(name) = name.unpack_str() else {
            return Err(errorf(format!(
                "in call to analysis_test(), parameter 'name' got value of type '{}', want 'string'",
                name.get_type()
            )));
        };
        let empty_list = || eval.heap().alloc(starlark::values::list::AllocList::EMPTY);
        let attrs =
            attrs.unwrap_or_else(|| eval.heap().alloc(starlark::values::dict::AllocDict::EMPTY));
        // PORT: blaze_packages' `Sequence.cast` reads lists and tuples; a range is passed as the
        // list of its elements, which `Sequence.cast` of the range sees one by one.
        let range_as_list =
            |v: Value<'v>, eval: &mut Evaluator<'v, '_, '_>| -> starlark::Result<Value<'v>> {
                if v.get_type() != "range" {
                    return Ok(v);
                }
                let elems: Vec<Value<'v>> = v.iterate(eval.heap())?.collect();
                Ok(eval.heap().alloc(starlark::values::list::AllocList(elems)))
            };
        let fragments = fragments.unwrap_or_else(empty_list);
        let toolchains = toolchains.unwrap_or_else(empty_list);
        let fragments = range_as_list(fragments, eval)?;
        let toolchains = range_as_list(toolchains, eval)?;
        let attr_values = attr_values
            .unwrap_or_else(|| eval.heap().alloc(starlark::values::dict::AllocDict::EMPTY));
        blaze_packages::starlark_defs::rule_class_functions::analysis_test(
            eval,
            name,
            implementation,
            attrs,
            fragments,
            toolchains,
            attr_values,
        )?;
        Ok(NoneType)
    }
}

/// Registers the analysis-capable `testing` module and the testing providers
/// (`AnalysisTestResultInfo` with its constructor). The lead substitutes these for W3's
/// `blaze_packages::starlark_defs::testing::register_testing` / `register_testing_providers`
/// entries; `InstrumentedFilesInfo` and `AnalysisFailureInfo` keep W3's values (no
/// constructor / unported).
// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingBootstrap.java:46 TestingBootstrap.addBindingsToBuilder
pub fn register_testing_toplevels(builder: &mut GlobalsBuilder) {
    builder.set("testing", StarlarkTestingModule);
    builder.set(
        "AnalysisTestResultInfo",
        analysis_test_result_info_provider(),
    );
}
