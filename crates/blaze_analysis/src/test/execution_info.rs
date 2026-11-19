//! Ports of `analysis/test/ExecutionInfo.java` (`testing.ExecutionInfo`) and
//! `analysis/test/AnalysisTestResultInfo.java`.

use std::fmt;

use allocative::Allocative;
use blaze_packages::starlark_defs::builtin_providers::{
    ANALYSIS_TEST_RESULT_INFO, BuiltinConstructor, BuiltinProvider, BuiltinProviderData,
    BuiltinProviderId,
};
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::info::InfoProviderKey;
use crate::providers::default_info::named_args;
use crate::providers::run_environment_info::dict_cast_string_string;
use crate::rule::rule_context::DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME;

pub use blaze_packages::starlark_defs::testing::EXECUTION_INFO;

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// This provider can be implemented by rules which need special environments to run in
/// (especially tests).
// Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:32 ExecutionInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExecutionInfo {
    #[allocative(skip)]
    execution_info: IndexMap<String, String>,
    exec_group: String,
}

starlark_simple_value!(ExecutionInfo);

impl ExecutionInfo {
    // Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:41 ExecutionInfo.<init>
    pub fn new(requirements: IndexMap<String, String>) -> ExecutionInfo {
        Self::with_exec_group(requirements, DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME.to_owned())
    }

    // Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:45 ExecutionInfo.<init>
    pub fn with_exec_group(
        requirements: IndexMap<String, String>,
        exec_group: String,
    ) -> ExecutionInfo {
        ExecutionInfo {
            execution_info: requirements,
            exec_group,
        }
    }

    /// Returns a map to indicate special execution requirements, such as hardware platforms,
    /// web browsers, etc.
    // Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:57 ExecutionInfo.getExecutionInfo
    pub fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }

    /// Returns the name of the exec group used to run the test action.
    // Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:62 ExecutionInfo.getExecGroup
    pub fn get_exec_group(&self) -> &str {
        &self.exec_group
    }
}

impl fmt::Display for ExecutionInfo {
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr (legacy
    // fields, sorted by name; strings through Printer.java:225 appendQuoted). Oracle:
    // `struct(exec_group = "test", requirements = {"K": "à"})`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use blaze_starlark::printer::quote;
        let reqs: Vec<String> = self
            .execution_info
            .iter()
            .map(|(k, v)| format!("{}: {}", quote(k), quote(v)))
            .collect();
        write!(
            f,
            "struct(exec_group = {}, requirements = {{{}}})",
            quote(&self.exec_group),
            reqs.join(", ")
        )
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/ExecutionInfoApi.java:33 ExecutionInfoApi
#[starlark_value(type = "ExecutionInfo")]
impl<'v> StarlarkValue<'v> for ExecutionInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "requirements" => {
                let entries: Vec<(&str, &str)> = self
                    .execution_info
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                Some(heap.alloc(starlark::values::dict::AllocDict(entries)))
            }
            "exec_group" => Some(heap.alloc(self.exec_group.as_str())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "requirements" | "exec_group")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["exec_group".to_owned(), "requirements".to_owned()]
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(EXECUTION_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// `testing.ExecutionInfo(requirements = {}, exec_group = 'test')`.
// Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:77 ExecutionInfo.ExecutionInfoProvider.constructor
pub fn execution_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let [requirements, exec_group] = named_args(
        "ExecutionInfo",
        ["requirements", "exec_group"],
        2,
        args,
        eval,
    )?;
    let requirements = match requirements {
        Some(v) => dict_cast_string_string(v, "requirements")?,
        None => IndexMap::new(),
    };
    let exec_group = match exec_group {
        None => DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME.to_owned(),
        Some(v) => v.unpack_str().map(str::to_owned).ok_or_else(|| {
            errorf(format!(
                "in call to ExecutionInfo(), parameter 'exec_group' got value of type '{}', want 'string'",
                v.get_type()
            ))
        })?,
    };
    Ok(eval
        .heap()
        .alloc(ExecutionInfo::with_exec_group(requirements, exec_group)))
}

/// Java `ExecutionInfo.PROVIDER` with its constructor.
// Java: com/google/devtools/build/lib/analysis/test/ExecutionInfo.java:35 ExecutionInfo.PROVIDER
pub const fn execution_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: EXECUTION_INFO,
        constructor: BuiltinConstructor::Native(execution_info_constructor),
    })
}

/// Encapsulates the result of analysis-phase testing. Build targets which return an instance of
/// this provider signal to the build system that it should generate a 'stub' test executable
/// which generates the equivalent test result.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:30 AnalysisTestResultInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct AnalysisTestResultInfo {
    success: bool,
    message: String,
}

starlark_simple_value!(AnalysisTestResultInfo);

impl AnalysisTestResultInfo {
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:39 AnalysisTestResultInfo.<init>
    pub fn new(success: bool, message: String) -> AnalysisTestResultInfo {
        AnalysisTestResultInfo { success, message }
    }

    // Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:50 AnalysisTestResultInfo.getSuccess
    pub fn get_success(&self) -> bool {
        self.success
    }

    // Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:55 AnalysisTestResultInfo.getMessage
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AnalysisTestResultInfo {
    // Java: StarlarkValue.repr default (Info without repr override).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.analysis.test.AnalysisTestResultInfo>"
        )
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/AnalysisTestResultInfoApi.java:28 AnalysisTestResultInfoApi
#[starlark_value(type = "AnalysisTestResultInfo")]
impl<'v> StarlarkValue<'v> for AnalysisTestResultInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "success" => Some(Value::new_bool(self.success)),
            "message" => Some(heap.alloc(self.message.as_str())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "success" | "message")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["message".to_owned(), "success".to_owned()]
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(ANALYSIS_TEST_RESULT_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// `AnalysisTestResultInfo(success, message)`.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:65 AnalysisTestResultInfo.TestResultInfoProvider.testResultInfo
pub fn analysis_test_result_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let [success, message] = named_args(
        "AnalysisTestResultInfo",
        ["success", "message"],
        0,
        args,
        eval,
    )?;
    let success = match success {
        Some(v) => v.unpack_bool().ok_or_else(|| {
            errorf(format!(
                "in call to AnalysisTestResultInfo(), parameter 'success' got value of type '{}', want 'bool'",
                v.get_type()
            ))
        })?,
        None => {
            return Err(errorf(
                "AnalysisTestResultInfo() missing 1 required named argument: success".to_owned(),
            ));
        }
    };
    let message = match message {
        Some(v) => v.unpack_str().map(str::to_owned).ok_or_else(|| {
            errorf(format!(
                "in call to AnalysisTestResultInfo(), parameter 'message' got value of type '{}', want 'string'",
                v.get_type()
            ))
        })?,
        None => {
            return Err(errorf(
                "AnalysisTestResultInfo() missing 1 required named argument: message".to_owned(),
            ));
        }
    };
    Ok(eval
        .heap()
        .alloc(AnalysisTestResultInfo::new(success, message)))
}

/// Java `AnalysisTestResultInfo.STARLARK_CONSTRUCTOR` with its constructor.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:28 AnalysisTestResultInfo.STARLARK_CONSTRUCTOR
pub const fn analysis_test_result_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: ANALYSIS_TEST_RESULT_INFO,
        constructor: BuiltinConstructor::Native(analysis_test_result_info_constructor),
    })
}

#[cfg(test)]
mod byte_strings_tests {
    use super::*;
    use blaze_util::string_encoding::latin1_decode;

    // Byte-strings review 2026-11-20 S1-3. Oracle (bazel 9.3.2):
    // `repr(testing.ExecutionInfo({"K": v}, exec_group = "g" + v))` with v = "à🐷\377" prints
    // `struct(exec_group = "g<v>", requirements = {"K": "<v>"})`, v as raw bytes; without
    // exec_group it is `"test"`.
    #[test]
    fn repr_is_struct_with_latin1_chars() {
        let v = latin1_decode(b"\xc3\xa0\xf0\x9f\x90\xb7\xff").into_owned();
        let reqs = IndexMap::from([("K".to_string(), v.clone())]);
        let info = ExecutionInfo::with_exec_group(reqs.clone(), format!("g{v}"));
        assert_eq!(
            info.to_string(),
            format!("struct(exec_group = \"g{v}\", requirements = {{\"K\": \"{v}\"}})")
        );
        assert_eq!(
            ExecutionInfo::new(reqs).to_string(),
            format!("struct(exec_group = \"test\", requirements = {{\"K\": \"{v}\"}})")
        );
    }
}
