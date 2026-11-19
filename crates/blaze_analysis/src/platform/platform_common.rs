//! The analysis-capable `platform_common` module (Java `PlatformCommon`): the same members as
//! the loading-phase `blaze_packages::starlark_defs::platform_common`, whose `ToolchainInfo` and
//! `TemplateVariableInfo` constructors create analysis infos.

use std::fmt;

use allocative::Allocative;
use blaze_packages::starlark_defs::builtin_providers::BuiltinProvider;
use blaze_packages::starlark_defs::platform_common::{
    constraint_setting_info, constraint_value_info, platform_info,
};
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::platform::{template_variable_info, toolchain_info};

/// The `platform_common` module (Java `PlatformCommon`).
// Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:26 PlatformCommon
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PlatformCommon;

starlark_simple_value!(PlatformCommon);

impl fmt::Display for PlatformCommon {
    // Java: StarlarkValue.repr default (PlatformCommon has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.rules.platform.PlatformCommon>"
        )
    }
}

/// The `structField` methods of `PlatformCommonApi`.
const PLATFORM_COMMON_FIELDS: [&str; 5] = [
    "TemplateVariableInfo",
    "ToolchainInfo",
    "PlatformInfo",
    "ConstraintSettingInfo",
    "ConstraintValueInfo",
];

impl PlatformCommon {
    fn field(name: &str) -> Option<BuiltinProvider> {
        match name {
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:44 PlatformCommon.getMakeVariableProvider
            "TemplateVariableInfo" => Some(template_variable_info::provider()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:49 PlatformCommon.getToolchainInfoConstructor
            "ToolchainInfo" => Some(toolchain_info::provider()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:29 PlatformCommon.getPlatformInfoConstructor
            "PlatformInfo" => Some(platform_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:34 PlatformCommon.getConstraintSettingInfoConstructor
            "ConstraintSettingInfo" => Some(constraint_setting_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:39 PlatformCommon.getConstraintValueInfoConstructor
            "ConstraintValueInfo" => Some(constraint_value_info()),
            _ => None,
        }
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/PlatformCommonApi.java:24 PlatformCommonApi
#[starlark_value(type = "platform_common")]
impl<'v> StarlarkValue<'v> for PlatformCommon {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        PlatformCommon::field(attribute).map(|p| heap.alloc(p))
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        PlatformCommon::field(attribute).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        PLATFORM_COMMON_FIELDS
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "unhashable type: 'platform_common'"
        )))
    }
}

/// Registers the analysis-capable `platform_common` (Java `PlatformBootstrap`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/PlatformBootstrap.java:33 PlatformBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_platform_common(builder: &mut GlobalsBuilder) {
    const platform_common: PlatformCommon = PlatformCommon;
}

#[cfg(test)]
mod tests {
    use starlark::environment::{Globals, GlobalsBuilder, Module};
    use starlark::eval::Evaluator;
    use starlark::syntax::{AstModule, Dialect};

    use super::register_platform_common;

    fn eval(code: &str) -> Result<String, String> {
        let globals: Globals = GlobalsBuilder::standard()
            .with(register_platform_common)
            .build();
        let ast = AstModule::parse("test.bzl", code.to_owned(), &Dialect::Extended)
            .map_err(|e| e.to_string())?;
        Module::with_temp_heap(|module| {
            let mut evaluator = Evaluator::new(&module);
            let v = evaluator
                .eval_module(ast, &globals)
                .map_err(|e| e.to_string())?;
            Ok(v.unpack_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_str()))
        })
    }

    #[test]
    fn platform_common_members() {
        // Expected values recorded from the oracle for the loading-phase module (W3); the
        // analysis-phase module has the same members.
        assert_eq!(
            eval(
                r#"
def _p(x):
    return "%s|%s" % (type(x), repr(x))
",".join([
    _p(platform_common), str(dir(platform_common)),
    _p(platform_common.ToolchainInfo), _p(platform_common.TemplateVariableInfo),
    _p(platform_common.PlatformInfo),
])
"#
            )
            .unwrap(),
            "platform_common|<unknown object com.google.devtools.build.lib.rules.platform.PlatformCommon>,\
             [\"ConstraintSettingInfo\", \"ConstraintValueInfo\", \"PlatformInfo\", \"TemplateVariableInfo\", \"ToolchainInfo\"],\
             Provider|<function ToolchainInfo>,Provider|<function TemplateVariableInfo>,\
             Provider|<function PlatformInfo>"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfoTest.java toolchainInfo_equalsTester
    #[test]
    fn toolchain_info_equals_tester() {
        assert_eq!(
            eval(
                r#"
a = platform_common.ToolchainInfo(foo = "val1", bar = "val2")
b = platform_common.ToolchainInfo(foo = "val1", bar = "val2")
c = platform_common.ToolchainInfo(foo = "val1", bar = "val3")
"%s %s %s %s" % (a == b, a == c, type(a), repr(a))
"#
            )
            .unwrap(),
            "True False ToolchainInfo struct(bar = \"val2\", foo = \"val1\")"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/TemplateVariableInfoTest.java templateVariableInfoConstructor
    #[test]
    fn template_variable_info_constructor() {
        assert_eq!(
            eval(
                r#"
t = platform_common.TemplateVariableInfo({"cherry": "ontop"})
u = platform_common.TemplateVariableInfo(vars = {"cherry": "ontop"})
"%s %s %s %s" % (type(t), t.variables["cherry"], u.variables, t == t)
"#
            )
            .unwrap(),
            "TemplateVariableInfo ontop {\"cherry\": \"ontop\"} True"
        );
        let e = eval("platform_common.TemplateVariableInfo({1: \"x\"})\n").unwrap_err();
        assert!(
            e.contains("got dict<int, string> for 'vars', want dict<string, string>"),
            "{e}"
        );
    }

    // The BuildViewTestCase tests of ToolchainInfoTest and TemplateVariableInfoTest live in
    // crates/blaze_sky_top/src/tests/{toolchain_info_test,template_variable_info_test}.rs.
}
