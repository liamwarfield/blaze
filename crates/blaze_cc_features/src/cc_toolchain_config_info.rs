//! Port of `com.google.devtools.build.lib.rules.cpp.CcToolchainConfigInfo`.

use std::sync::Arc;

use crate::actions::OS;
use crate::cc_toolchain_features::{
    ActionConfig, ArtifactNamePatternMapper, ArtifactNamePatternMapperBuilder, Feature,
};
use crate::cc_toolchain_features_lib::{
    action_config_from_starlark, artifact_name_pattern_from_starlark, feature_from_starlark,
};
use crate::errors::EvalException;
use crate::starlark_info::{StarlarkInfo, StarlarkInfoValue, sequence_cast};

/// Information describing C++ toolchain derived from CROSSTOOL file: a view of the rules_cc
/// `CcToolchainConfigInfo` provider instance.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainConfigInfo.java:39 CcToolchainConfigInfo
// PORT: provider lookup (`get(ConfiguredTarget)`, `PROVIDER`, `RULES_CC_PROVIDER`) belongs to the
// analysis layer; the provider keys are
// `@_builtins//:common/cc/toolchain_config/cc_toolchain_config_info.bzl%CcToolchainConfigInfo` and
// `@rules_cc+//cc/private/toolchain_config:cc_toolchain_config_info.bzl%CcToolchainConfigInfo`.
#[derive(Clone, Debug)]
pub struct CcToolchainConfigInfo {
    actual: Arc<StarlarkInfo>,
}

impl CcToolchainConfigInfo {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainConfigInfo.java:56 CcToolchainConfigInfo.<init>
    pub fn new(actual: Arc<StarlarkInfo>) -> CcToolchainConfigInfo {
        CcToolchainConfigInfo { actual }
    }

    pub fn get_actual(&self) -> &StarlarkInfo {
        &self.actual
    }

    fn get_list(&self, field: &str, what: &str) -> Result<Vec<Arc<StarlarkInfo>>, EvalException> {
        let value = self
            .actual
            .get_value(field)
            .unwrap_or_else(|| panic!("NullPointerException: {field}"));
        Ok(sequence_cast(value, "struct", what, false)?
            .iter()
            .map(|v| match v {
                StarlarkInfoValue::Info(i) => i.clone(),
                _ => unreachable!(),
            })
            .collect())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainConfigInfo.java:60 CcToolchainConfigInfo.getActionConfigs
    pub fn get_action_configs(&self) -> Result<Vec<ActionConfig>, EvalException> {
        let exec_os_name = self.actual.get_string_value("_exec_os_DO_NOT_USE")?;
        let exec_os = OS::value_of(&exec_os_name).unwrap_or_else(|| {
            panic!(
                "IllegalArgumentException: No enum constant com.google.devtools.build.lib.util.OS.{exec_os_name}"
            )
        });
        let mut action_config_builder = Vec::new();
        for action_config in self.get_list("_action_configs_DO_NOT_USE", "_action_configs")? {
            action_config_builder.push(action_config_from_starlark(&action_config, exec_os)?);
        }
        Ok(action_config_builder)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainConfigInfo.java:71 CcToolchainConfigInfo.getFeatures
    pub fn get_features(&self) -> Result<Vec<Feature>, EvalException> {
        let mut feature_builder = Vec::new();
        for feature in self.get_list("_features_DO_NOT_USE", "_features")? {
            feature_builder.push(feature_from_starlark(&feature)?);
        }
        Ok(feature_builder)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainConfigInfo.java:80 CcToolchainConfigInfo.getArtifactNamePatterns
    pub fn get_artifact_name_patterns(&self) -> Result<ArtifactNamePatternMapper, EvalException> {
        let mut artifact_name_pattern_builder = ArtifactNamePatternMapperBuilder::new();
        for artifact_name_pattern in self.get_list(
            "_artifact_name_patterns_DO_NOT_USE",
            "_artifact_name_patterns",
        )? {
            artifact_name_pattern_from_starlark(
                &artifact_name_pattern,
                &mut |category, prefix, extension| {
                    artifact_name_pattern_builder.add_override(category, prefix, extension)
                },
            )?;
        }
        Ok(artifact_name_pattern_builder.build())
    }
}
