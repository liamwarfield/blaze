//! The `config_common` module, `toolchain_type` values and `FeatureFlagInfo`.
//!
//! Java: rules/config/ConfigStarlarkCommon.java (starlarkbuildapi/config/
//! ConfigStarlarkCommonApi.java), analysis/config/ToolchainTypeRequirement.java as the Starlark
//! value `toolchain_type` (starlarkbuildapi/config/StarlarkToolchainTypeRequirement.java) and
//! rules/config/ConfigFeatureFlagProvider.java (`STARLARK_CONSTRUCTOR`).

use std::fmt;

use allocative::Allocative;
use blaze_starlark::label::LabelValue;
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
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::rule_class::ToolchainTypeRequirement;
use crate::starlark_defs::builtin_providers::BuiltinConstructor;
use crate::starlark_defs::builtin_providers::BuiltinProviderData;
use crate::starlark_defs::builtin_providers::BuiltinProviderId;
use crate::starlark_defs::builtin_providers::builtin_provider_type;
use crate::starlark_defs::util::errorf;
use crate::types::LabelConverter;

// -------------------------------------------------------------------------------------------
// FeatureFlagInfo

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:80 ConfigFeatureFlagProvider.Constructor
pub const FEATURE_FLAG_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "FeatureFlagInfo",
    value_class: "ConfigFeatureFlagProvider",
};

builtin_provider_type!(
    /// `FeatureFlagInfo` (Java `ConfigFeatureFlagProvider.STARLARK_CONSTRUCTOR`, whose Starlark
    /// type is `FeatureFlagInfo`).
    ConfigFeatureFlagProviderConstructor,
    "FeatureFlagInfo"
);

/// Java `ConfigFeatureFlagProvider.STARLARK_CONSTRUCTOR`.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:47 ConfigFeatureFlagProvider.STARLARK_CONSTRUCTOR
pub const fn feature_flag_info() -> ConfigFeatureFlagProviderConstructor {
    ConfigFeatureFlagProviderConstructor {
        provider: BuiltinProviderData {
            id: FEATURE_FLAG_INFO,
            // PORT: analysis-time (needs ConfigFeatureFlagProvider instances).
            constructor: BuiltinConstructor::Unported(
                "ConfigFeatureFlagProvider.Constructor.selfcall",
            ),
        },
    }
}

// -------------------------------------------------------------------------------------------
// toolchain_type

/// A `ToolchainTypeRequirement` as a Starlark value (type `toolchain_type`).
#[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkToolchainTypeRequirement(#[allocative(skip)] pub ToolchainTypeRequirement);

starlark_simple_value!(StarlarkToolchainTypeRequirement);

impl fmt::Display for StarlarkToolchainTypeRequirement {
    // Java: StarlarkValue.repr default (the record has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.analysis.config.ToolchainTypeRequirement>"
        )
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/config/StarlarkToolchainTypeRequirement.java:24 StarlarkToolchainTypeRequirement
#[starlark_value(type = "toolchain_type")]
impl<'v> StarlarkValue<'v> for StarlarkToolchainTypeRequirement {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/starlarkbuildapi/config/StarlarkToolchainTypeRequirement.java:30 StarlarkToolchainTypeRequirement.toolchainType
            "toolchain_type" => Some(heap.alloc(LabelValue(self.0.toolchain_type.clone()))),
            // Java: com/google/devtools/build/lib/starlarkbuildapi/config/StarlarkToolchainTypeRequirement.java:36 StarlarkToolchainTypeRequirement.mandatory
            "mandatory" => Some(Value::new_bool(self.0.mandatory)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "toolchain_type" | "mandatory")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["mandatory".to_owned(), "toolchain_type".to_owned()]
    }

    // Java: record ToolchainTypeRequirement equality (all components).
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkToolchainTypeRequirement>()
            .is_some_and(|o| o.0 == self.0))
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'toolchain_type'"))
    }
}

/// Java `rawToolchain instanceof ToolchainTypeRequirement`: the requirement of a
/// `config_common.toolchain_type(...)` value.
pub fn toolchain_type_requirement(v: Value) -> Option<ToolchainTypeRequirement> {
    v.downcast_ref::<StarlarkToolchainTypeRequirement>()
        .map(|r| r.0.clone())
}

// -------------------------------------------------------------------------------------------
// config_common

/// The `config_common` module (Java `ConfigStarlarkCommon`).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConfigStarlarkCommon;

starlark_simple_value!(ConfigStarlarkCommon);

impl fmt::Display for ConfigStarlarkCommon {
    // Java: StarlarkValue.repr default (ConfigStarlarkCommon has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.rules.config.ConfigStarlarkCommon>"
        )
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/config/ConfigStarlarkCommonApi.java:33 ConfigStarlarkCommonApi
#[starlark_value(type = "config_common")]
impl<'v> StarlarkValue<'v> for ConfigStarlarkCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("config_common", config_common_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/config/ConfigStarlarkCommon.java:31 ConfigStarlarkCommon.getConfigFeatureFlagProviderConstructor
            "FeatureFlagInfo" => Some(heap.alloc(feature_flag_info())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "FeatureFlagInfo"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["FeatureFlagInfo".to_owned()]
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'config_common'"))
    }
}

#[starlark_module]
fn config_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/config/ConfigStarlarkCommon.java:36 ConfigStarlarkCommon.createConfigFeatureFlagTransitionFactory
    fn config_feature_flag_transition<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] attribute: &str,
    ) -> starlark::Result<NoneType> {
        let _ = (this, attribute);
        // PORT: ConfigFeatureFlagTransitionFactory (a configuration transition) is not ported.
        blaze_util::unported!("ConfigStarlarkCommon.createConfigFeatureFlagTransitionFactory")
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigStarlarkCommon.java:41 ConfigStarlarkCommon.toolchainType
    fn toolchain_type<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] name: Value<'v>,
        #[starlark(require = named)] mandatory: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkToolchainTypeRequirement> {
        let _ = this;
        // Java: the @Param allowedTypes checks of ConfigStarlarkCommonApi.toolchainType.
        if name.unpack_str().is_none() && name.downcast_ref::<LabelValue>().is_none() {
            return Err(errorf(format!(
                "in call to toolchain_type(), parameter 'name' got value of type '{}', want 'string or Label'",
                name.get_type()
            )));
        }
        let mandatory = match mandatory {
            None => true,
            Some(m) => match m.unpack_bool() {
                Some(b) => b,
                None => {
                    return Err(errorf(format!(
                        "in call to toolchain_type(), parameter 'mandatory' got value of type '{}', want 'bool'",
                        m.get_type()
                    )));
                }
            },
        };

        let label = if let Some(name_label) = name.downcast_ref::<LabelValue>() {
            name_label.label().clone()
        } else {
            let name = name.unpack_str().unwrap();
            let converter = LabelConverter::for_bzl_evaluating_thread(eval)
                .map_err(|e| errorf(e.get_message()))?;
            match converter.convert(name) {
                Ok(label) => label,
                Err(e) => {
                    return Err(errorf(format!(
                        "Unable to parse toolchain_type label '{}': {}",
                        name,
                        e.get_message()
                    )));
                }
            }
        };

        // Java: ToolchainTypeRequirement.builder(label).mandatory(mandatory).build()
        Ok(StarlarkToolchainTypeRequirement(ToolchainTypeRequirement {
            toolchain_type: label,
            mandatory,
            ignore_if_invalid: false,
        }))
    }
}

/// Registers `config_common` (Java `ConfigBootstrap`; `config` and `ConfigGlobalLibrary`
/// belong to other files).
// Java: com/google/devtools/build/lib/starlarkbuildapi/config/ConfigBootstrap.java:39 ConfigBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_config_common(builder: &mut GlobalsBuilder) {
    const config_common: ConfigStarlarkCommon = ConfigStarlarkCommon;
}

#[cfg(test)]
mod tests {
    use crate::starlark_defs::provider::test_support::error_message;
    use crate::starlark_defs::provider::test_support::exec_bzl;
    use crate::starlark_defs::provider::test_support::string_global;

    // Expected values recorded from the oracle (Bazel 9.3.2), with the .bzl at //p:defs.bzl
    // there and //pkg:<file> here.

    #[test]
    fn config_common_members() {
        let m = exec_bzl(
            "config_common_test_1.bzl",
            r#"
def _p(x):
    return "%s|%s" % (type(x), repr(x))
result = ",".join([
    _p(config_common), str(dir(config_common)), _p(config_common.FeatureFlagInfo),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "config_common|<unknown object com.google.devtools.build.lib.rules.config.ConfigStarlarkCommon>,\
             [\"FeatureFlagInfo\", \"config_feature_flag_transition\", \"toolchain_type\"],\
             FeatureFlagInfo|<function FeatureFlagInfo>"
        );
    }

    #[test]
    fn toolchain_type_values() {
        let m = exec_bzl(
            "config_common_test_2.bzl",
            r#"
tt = config_common.toolchain_type("//pkg:tt", mandatory = False)
tt2 = config_common.toolchain_type(Label("//pkg:tt"))
result = "|".join([
    type(tt), repr(tt), str(tt.toolchain_type), str(tt.mandatory), str(dir(tt)),
    str(tt == config_common.toolchain_type("//pkg:tt", mandatory = False)),
    str(tt == config_common.toolchain_type(":tt")),
    str(tt2.toolchain_type), str(tt2.mandatory),
])
"#,
        )
        .unwrap();
        // The oracle probe printed these with print() (Label.debugPrint: display form "//p:tt");
        // str() is Label.str, which under the default
        // --incompatible_unambiguous_label_stringification gives "@@//pkg:tt" (Label.java:740).
        assert_eq!(
            string_global(&m, "result"),
            "toolchain_type|<unknown object com.google.devtools.build.lib.analysis.config.ToolchainTypeRequirement>|\
             @@//pkg:tt|False|[\"mandatory\", \"toolchain_type\"]|True|False|@@//pkg:tt|True"
        );
        let tt = m.get("tt").unwrap();
        let req = super::toolchain_type_requirement(tt.value()).unwrap();
        assert_eq!(req.toolchain_type.get_canonical_form(), "//pkg:tt");
        assert!(!req.mandatory);
        assert!(!req.ignore_if_invalid);
    }

    #[test]
    fn toolchain_type_errors() {
        let e = error_message(exec_bzl(
            "config_common_test_3.bzl",
            "x = config_common.toolchain_type(1)\n",
        ));
        assert!(
            e.contains(
                "in call to toolchain_type(), parameter 'name' got value of type 'int', want 'string or Label'"
            ),
            "{e}"
        );
        let e = error_message(exec_bzl(
            "config_common_test_4.bzl",
            "x = config_common.toolchain_type(\"//a:b:c\")\n",
        ));
        assert!(
            e.contains(
                "Unable to parse toolchain_type label '//a:b:c': invalid target name 'b:c': target names may not contain ':'"
            ),
            "{e}"
        );
    }
}
