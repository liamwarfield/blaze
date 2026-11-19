//! Port of `analysis/TemplateVariableInfo.java`.
//!
//! PORT: Java keeps it in `com.google.devtools.build.lib.analysis`; it lives with the platform
//! providers here because `platform_common.TemplateVariableInfo` exposes it.

use std::fmt;

use allocative::Allocative;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::{
    BuiltinConstructor, BuiltinProvider, BuiltinProviderData, BuiltinProviderId,
};
use blaze_packages::starlark_defs::platform_common::TEMPLATE_VARIABLE_INFO;
use blaze_starlark::structs::StructureMarker;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::dict::{AllocDict, DictRef};
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value,
};

use crate::info::{InfoProviderKey, downcast_info};

/// Provides access to make variables from the current fragments.
// Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:31 TemplateVariableInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TemplateVariableInfo {
    #[allocative(skip)]
    variables: IndexMap<String, String>,
    /// Java `NativeInfo.location` (the caller location of the constructor, or builtin).
    #[allocative(skip)]
    creation_location: Option<blaze_starlark::Location>,
}

starlark_simple_value!(TemplateVariableInfo);

impl TemplateVariableInfo {
    // Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:53 TemplateVariableInfo.<init>
    pub fn new(
        variables: IndexMap<String, String>,
        creation_location: Option<blaze_starlark::Location>,
    ) -> TemplateVariableInfo {
        TemplateVariableInfo {
            variables,
            creation_location,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:64 TemplateVariableInfo.getVariables
    pub fn get_variables(&self) -> &IndexMap<String, String> {
        &self.variables
    }

    /// Java `PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        TEMPLATE_VARIABLE_INFO.key()
    }

    /// Java `((TemplateVariableInfo) info).getVariables()` for a frozen info.
    pub fn variables_of(info: &OwnedFrozenValue) -> IndexMap<String, String> {
        downcast_info::<TemplateVariableInfo>(info)
            .expect("ClassCastException: not a TemplateVariableInfo")
            .get_variables()
            .clone()
    }

    /// Java `NativeInfo.getCreationLocation` (`None` is `Location.BUILTIN`).
    pub fn get_creation_location(&self) -> Option<&blaze_starlark::Location> {
        self.creation_location.as_ref()
    }
}

impl fmt::Display for TemplateVariableInfo {
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr (legacy
    // fields, sorted by name; strings through Printer.java:225 appendQuoted)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct(variables = {{")?;
        for (i, (k, v)) in self.variables.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(
                f,
                "{}: {}",
                blaze_starlark::printer::quote(k),
                blaze_starlark::printer::quote(v)
            )?;
        }
        write!(f, "}})")
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/TemplateVariableInfoApi.java:42 TemplateVariableInfoApi
#[starlark_value(type = "TemplateVariableInfo")]
impl<'v> StarlarkValue<'v> for TemplateVariableInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "variables" => Some(heap.alloc(AllocDict(
                self.variables.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            ))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "variables"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["variables".to_owned()]
    }

    // Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:69 TemplateVariableInfo.equals
    // (identity; starlark-rust compares pointers before calling `equals`).
    fn equals(&self, _other: Value<'v>) -> starlark::Result<bool> {
        Ok(false)
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(TEMPLATE_VARIABLE_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `TemplateVariableInfo.Provider.templateVariableInfo(vars, thread)`:
/// `@Param(name = "vars", positional = true, named = true)`.
// Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:44 TemplateVariableInfo.Provider.templateVariableInfo
fn template_variable_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let positions: Vec<Value<'v>> = args.positions(eval.heap())?.collect();
    let names = args.names_map()?;
    let mut vars: Option<Value<'v>> = None;
    if positions.len() > 1 {
        return Err(errorf(format!(
            "TemplateVariableInfo() accepts no more than 1 positional argument but got {}",
            positions.len()
        )));
    }
    if let Some(v) = positions.first() {
        vars = Some(*v);
    }
    for (k, v) in names.iter() {
        if k.as_str() != "vars" {
            return Err(errorf(format!(
                "TemplateVariableInfo() got unexpected keyword argument: {}",
                k.as_str()
            )));
        }
        if vars.is_some() {
            return Err(errorf(
                "TemplateVariableInfo() got multiple values for argument 'vars'",
            ));
        }
        vars = Some(*v);
    }
    let Some(vars) = vars else {
        return Err(errorf(
            "TemplateVariableInfo() missing 1 required positional argument: vars",
        ));
    };
    let Some(dict) = DictRef::from_value(vars) else {
        return Err(errorf(format!(
            "in call to TemplateVariableInfo(), parameter 'vars' got value of type '{}', want 'dict'",
            vars.get_type()
        )));
    };
    // Java: Dict.noneableCast(vars, String.class, String.class, "vars").
    let mut vars_map = IndexMap::new();
    for (k, v) in dict.iter() {
        match (k.unpack_str(), v.unpack_str()) {
            (Some(ks), Some(vs)) => {
                vars_map.insert(ks.to_owned(), vs.to_owned());
            }
            _ => {
                return Err(errorf(format!(
                    "got dict<{}, {}> for 'vars', want dict<string, string>",
                    k.get_type(),
                    v.get_type()
                )));
            }
        }
    }
    // Java: thread.getCallerLocation().
    let location = blaze_starlark::env::caller_location(eval);
    Ok(eval
        .heap()
        .alloc(TemplateVariableInfo::new(vars_map, location)))
}

/// Java `TemplateVariableInfo.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:34 TemplateVariableInfo.PROVIDER
pub const fn provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: TEMPLATE_VARIABLE_INFO,
        constructor: BuiltinConstructor::Native(template_variable_info_constructor),
    })
}

#[cfg(test)]
mod byte_strings_tests {
    use super::*;
    use blaze_util::string_encoding::latin1_decode;

    // Byte-strings review 2026-11-20 S1-3. Oracle (bazel 9.3.2):
    // `repr(platform_common.TemplateVariableInfo({"K": "à🐷\377"}))` prints
    // `struct(variables = {"K": "` C3 A0 F0 9F 90 B7 FF `"})`: the bytes as they are.
    #[test]
    fn repr_keeps_latin1_chars() {
        let v = latin1_decode(b"\xc3\xa0\xf0\x9f\x90\xb7\xff").into_owned();
        let info = TemplateVariableInfo::new(IndexMap::from([("K".to_string(), v.clone())]), None);
        assert_eq!(
            info.to_string(),
            format!("struct(variables = {{\"K\": \"{v}\"}})")
        );
    }
}
