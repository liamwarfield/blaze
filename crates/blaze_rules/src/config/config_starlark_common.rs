//! The analysis-capable `config_common` module (Java `ConfigStarlarkCommon`): the same members as
//! the loading-phase `blaze_packages::starlark_defs::config_common`, whose `FeatureFlagInfo`
//! constructor creates analysis infos.

use std::fmt;

use allocative::Allocative;
use blaze_packages::starlark_defs::config_common::ConfigStarlarkCommon as LoadingConfigStarlarkCommon;
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::config::config_feature_flag_provider;

/// The `config_common` module (Java `ConfigStarlarkCommon`).
// Java: com/google/devtools/build/lib/rules/config/ConfigStarlarkCommon.java:29 ConfigStarlarkCommon
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
    // PORT: `toolchain_type` and `config_feature_flag_transition` are the loading module's
    // methods (one port of each Java method; neither reads its receiver).
    fn get_methods() -> Option<&'static Methods> {
        <LoadingConfigStarlarkCommon as StarlarkValue<'v>>::get_methods()
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/config/ConfigStarlarkCommon.java:31 ConfigStarlarkCommon.getConfigFeatureFlagProviderConstructor
            "FeatureFlagInfo" => {
                Some(heap.alloc(config_feature_flag_provider::starlark_constructor()))
            }
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
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "unhashable type: 'config_common'"
        )))
    }
}

/// Registers the analysis-capable `config_common` (Java `ConfigBootstrap`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/config/ConfigBootstrap.java:39 ConfigBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_config_common(builder: &mut GlobalsBuilder) {
    const config_common: ConfigStarlarkCommon = ConfigStarlarkCommon;
}
