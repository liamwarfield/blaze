//! Port of `rules/core/ValidateTarget.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_aspect_factory::ConfiguredAspectFactory;
use blaze_analysis::configured_rule_class_provider::CreateConfiguredTargetError;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configuredtargets::get_info;
use blaze_analysis::info::downcast_info;
use blaze_analysis::providers::output_group_info::{
    OutputGroupInfo, VALIDATION, VALIDATION_TOP_LEVEL,
};
use blaze_analysis::rule::RuleContext;
use blaze_cmdline::{Label, RepositoryName};
use blaze_packages::aspect::{AspectClass, AspectDefinition, AspectParameters, NativeAspectClass};
use blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO;

/// Non-recursive aspect that promotes `OutputGroupInfo.VALIDATION` outputs to
/// `OutputGroupInfo.VALIDATION_TOP_LEVEL`. By requesting the latter but not the former output
/// group, validations avoid blocking test execution. (Using `OutputGroupInfo.DEFAULT` would
/// accomplish that as well but would be overrideable with `--output_groups` flag.)
///
/// Name is chosen to make for semi-sensible "ValidateTarget" aspect events.
// Java: com/google/devtools/build/lib/rules/core/ValidateTarget.java:38 ValidateTarget
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidateTarget;

impl NativeAspectClass for ValidateTarget {
    fn get_name(&self) -> &str {
        "ValidateTarget"
    }

    // Java: com/google/devtools/build/lib/rules/core/ValidateTarget.java:41 ValidateTarget.getDefinition
    fn get_definition(&self, _aspect_parameters: &AspectParameters) -> Arc<AspectDefinition> {
        AspectDefinition::builder(AspectClass::Native(Arc::new(ValidateTarget)))
            .apply_to_files(true) // to grab validation outputs from file targets
            .build()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ConfiguredAspectFactory for ValidateTarget {
    // Java: com/google/devtools/build/lib/rules/core/ValidateTarget.java:48 ValidateTarget.create
    fn create(
        &self,
        _target_label: &Label,
        ct: &ConfiguredTarget,
        context: &mut RuleContext,
        _parameters: &AspectParameters,
        _tools_repository: &RepositoryName,
    ) -> Result<Option<ConfiguredAspect>, CreateConfiguredTargetError> {
        // OutputGroupInfo.get(ct)
        if let Some(info) = get_info(ct, &OUTPUT_GROUP_INFO.key()) {
            let output_group_info = downcast_info::<OutputGroupInfo>(&info)
                .expect("ClassCastException: OutputGroupInfo");
            let validations = output_group_info.get_output_group(VALIDATION);
            if !validations.is_empty() {
                let mut builder = ConfiguredAspect::builder(context);
                builder.add_output_group(VALIDATION_TOP_LEVEL, &validations);
                return Ok(builder.build()?);
            }
        }
        Ok(Some(ConfiguredAspect::NonApplicable))
    }
}
