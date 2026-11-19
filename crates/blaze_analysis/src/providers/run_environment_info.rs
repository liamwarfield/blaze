//! Port of `analysis/RunEnvironmentInfo.java`.

use std::fmt;

use allocative::Allocative;
use blaze_packages::starlark_defs::builtin_providers::{BuiltinProviderId, RUN_ENVIRONMENT_INFO};
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::dict::DictRef;
use starlark::values::list::AllocList;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, UnpackValue, Value, starlark_value,
};

use crate::info::InfoProviderKey;
use crate::providers::default_info::named_args;

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Provider containing any additional environment variables for use when running executables,
/// either in `bazel run` or `bazel test`.
// Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:33 RunEnvironmentInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RunEnvironmentInfo {
    #[allocative(skip)]
    environment: IndexMap<String, String>,
    inherited_environment: Vec<String>,
    should_error_on_non_executable_rule: bool,
}

starlark_simple_value!(RunEnvironmentInfo);

impl RunEnvironmentInfo {
    /// Constructs a new provider with the given fixed environment variables and variables to
    /// inherit from the client environment.
    // Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:44 RunEnvironmentInfo.<init>
    pub fn new(
        environment: IndexMap<String, String>,
        inherited_environment: Vec<String>,
        should_error_on_non_executable_rule: bool,
    ) -> RunEnvironmentInfo {
        RunEnvironmentInfo {
            environment,
            inherited_environment,
            should_error_on_non_executable_rule,
        }
    }

    /// Returns environment variables which should be set when the target advertising this
    /// provider is executed.
    // Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:63 RunEnvironmentInfo.getEnvironment
    pub fn get_environment(&self) -> &IndexMap<String, String> {
        &self.environment
    }

    /// Returns names of environment variables whose value should be inherited from the shell
    /// environment when the target advertising this provider is executed.
    // Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:72 RunEnvironmentInfo.getInheritedEnvironment
    pub fn get_inherited_environment(&self) -> &[String] {
        &self.inherited_environment
    }

    /// Returns whether advertising this provider on a non-executable (and thus non-test) rule
    /// should result in an error or a warning.
    // Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:81 RunEnvironmentInfo.shouldErrorOnNonExecutableRule
    pub fn should_error_on_non_executable_rule(&self) -> bool {
        self.should_error_on_non_executable_rule
    }
}

impl fmt::Display for RunEnvironmentInfo {
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr (legacy
    // fields, sorted by name; strings through Printer.java:225 appendQuoted). Oracle:
    // `struct(environment = {"K": "à"}, inherited_environment = ["à"])`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use blaze_starlark::printer::quote;
        let env: Vec<String> = self
            .environment
            .iter()
            .map(|(k, v)| format!("{}: {}", quote(k), quote(v)))
            .collect();
        let inherited: Vec<String> = self
            .inherited_environment
            .iter()
            .map(|s| quote(s))
            .collect();
        write!(
            f,
            "struct(environment = {{{}}}, inherited_environment = [{}])",
            env.join(", "),
            inherited.join(", ")
        )
    }
}

const FIELDS: [&str; 2] = ["environment", "inherited_environment"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/RunEnvironmentInfoApi.java:35 RunEnvironmentInfoApi
#[starlark_value(type = "RunEnvironmentInfo")]
impl<'v> StarlarkValue<'v> for RunEnvironmentInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "environment" => {
                let entries: Vec<(&str, &str)> = self
                    .environment
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                Some(heap.alloc(starlark::values::dict::AllocDict(entries)))
            }
            "inherited_environment" => Some(heap.alloc(AllocList(
                self.inherited_environment.iter().map(|s| s.as_str()),
            ))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        FIELDS.iter().map(|s| s.to_string()).collect()
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(RUN_ENVIRONMENT_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// Java `Dict.cast(x, String.class, String.class, what)`.
// Java: net/starlark/java/eval/Dict.java:614 Dict.cast
pub(crate) fn dict_cast_string_string(
    x: Value,
    what: &str,
) -> starlark::Result<IndexMap<String, String>> {
    let Some(d) = DictRef::from_value(x) else {
        return Err(errorf(format!(
            "for {what}, got {}, want dict",
            x.get_type()
        )));
    };
    let mut out = IndexMap::new();
    for (k, v) in d.iter() {
        let (Some(ks), Some(vs)) = (k.unpack_str(), v.unpack_str()) else {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<string, string>",
                k.get_type(),
                v.get_type()
            )));
        };
        out.insert(ks.to_owned(), vs.to_owned());
    }
    Ok(out)
}

/// Java `Sequence.cast(x, String.class, what)`.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub(crate) fn sequence_cast_strings(x: Value, what: &str) -> starlark::Result<Vec<String>> {
    let Some(seq) = UnpackListOrTuple::<Value>::unpack_value(x)? else {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    };
    let mut out = Vec::with_capacity(seq.items.len());
    for (i, elem) in seq.items.iter().enumerate() {
        match elem.unpack_str() {
            Some(s) => out.push(s.to_owned()),
            None => {
                return Err(errorf(format!(
                    "at index {i} of {what}, got element of type {}, want string",
                    elem.get_type()
                )));
            }
        }
    }
    Ok(out)
}

/// The `RunEnvironmentInfo(environment = {}, inherited_environment = [])` constructor.
// Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:98 RunEnvironmentInfo.RunEnvironmentInfoProvider.constructor
pub fn run_environment_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let [environment, inherited_environment] = named_args(
        "RunEnvironmentInfo",
        ["environment", "inherited_environment"],
        2,
        args,
        eval,
    )?;
    let environment = match environment {
        Some(v) => dict_cast_string_string(v, "environment")?,
        None => IndexMap::new(),
    };
    let inherited_environment = match inherited_environment {
        Some(v) => sequence_cast_strings(v, "inherited_environment")?,
        None => Vec::new(),
    };
    Ok(eval.heap().alloc(RunEnvironmentInfo::new(
        environment,
        inherited_environment,
        /* should_error_on_non_executable_rule= */ true,
    )))
}

#[cfg(test)]
mod byte_strings_tests {
    use super::*;
    use blaze_util::string_encoding::latin1_decode;

    // Byte-strings review 2026-11-20 S1-3. Oracle (bazel 9.3.2): `repr(RunEnvironmentInfo({"K": v},
    // [v]))` and `str(...)` with v = "à🐷\377" print
    // `struct(environment = {"K": "<v>"}, inherited_environment = ["<v>"])`, v as raw bytes.
    #[test]
    fn repr_is_struct_with_latin1_chars() {
        let v = latin1_decode(b"\xc3\xa0\xf0\x9f\x90\xb7\xff").into_owned();
        let info = RunEnvironmentInfo::new(
            IndexMap::from([("K".to_string(), v.clone())]),
            vec![v.clone()],
            false,
        );
        assert_eq!(
            info.to_string(),
            format!("struct(environment = {{\"K\": \"{v}\"}}, inherited_environment = [\"{v}\"])")
        );
    }
}
