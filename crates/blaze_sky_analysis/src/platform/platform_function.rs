//! Port of `analysis/platform/PlatformFunction.java`: validates that a label is a platform, then
//! requests its configured target, extracts the `PlatformInfo` and parses any
//! `PlatformInfo.flags()`.
//!
//! The PLATFORM SkyFunction is registered by the configuration functions
//! (`blaze_build_config::skyframe::register_configuration_functions`) through the
//! [`PlatformValueProvider`] seam; [`PlatformFunction`] is its implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_build_config::parsed_flags::ParsedFlagsKey;
use blaze_cmdline::LabelPackageContext;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_build_config::platform::{PlatformValue, PlatformValueKey, PlatformValueProvider};
use blaze_cmdline::Label;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::{ComputeError, Environment, SkyError, SkyFunctionException, Transience};

use crate::toolchains::lookup_error::catch_configured_value_creation_exception;
use crate::toolchains::platform_keys::empty_options_configuration_key;
use crate::toolchains::platform_lookup_util::{
    has_platform_info, invalid_platform_exception, invalid_platform_exception_no_such_thing,
    invalid_platform_exception_with_cause,
};

/// Java `PlatformFunction` (the PLATFORM SkyFunction).
// Java: com/google/devtools/build/lib/analysis/platform/PlatformFunction.java:43 PlatformFunction
pub struct PlatformFunction {
    /// The command's analysis services (PLATFORM runs under the configuration functions'
    /// evaluator client, so the services are captured here like the other analysis
    /// functions'; PlatformFunction itself reads none of them).
    #[allow(dead_code)]
    services: Arc<AnalysisServices>,
}

impl PlatformFunction {
    pub fn new(services: Arc<AnalysisServices>) -> Self {
        PlatformFunction { services }
    }

    /// Returns the [`ConfiguredTargetKey`] requested when evaluating the given platform.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformFunction.java:46 PlatformFunction.configuredTargetDep
    pub fn configured_target_dep(platform_label: &Label) -> ConfiguredTargetKey {
        // Platforms do not rely on the configuration. Use a dummy blank configuration to reduce
        // the number of skyframe nodes created.
        ConfiguredTargetKey::builder()
            .set_label(platform_label.clone())
            .set_configuration_key(Some(empty_options_configuration_key()))
            .build()
    }

    /// Computes the full `PlatformInfo` of a platform (Java `PlatformFunction.compute` up to the
    /// flag parsing). `Ok(None)` where Java returns null.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformFunction.java:57 PlatformFunction.compute
    pub fn compute_platform_info(
        platform_label: &Label,
        env: &mut Environment<'_>,
    ) -> Result<Option<PlatformInfo>, ComputeError> {
        let pkg_id = platform_label.get_package_identifier();

        // Load the Package first to verify the Target. The ConfiguredTarget should not be loaded
        // until after verification. See https://github.com/bazelbuild/bazel/pull/10307.
        //
        // In distributed analysis, these packages will be duplicated across shards.
        let pkg_value = match env
            .get_value_or_throw::<PackageKey, NoSuchPackageException>(PackageKey(pkg_id.clone()))?
        {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(None),
            Err(e) => {
                return Err(platform_function_exception(
                    invalid_platform_exception_no_such_thing(NoSuchThingException::Package(e)),
                ));
            }
        };
        let target = match pkg_value
            .get_package()
            .get_target(platform_label.get_name())
        {
            Ok(t) => t.clone(),
            Err(e) => {
                return Err(platform_function_exception(
                    invalid_platform_exception_no_such_thing(NoSuchThingException::Target(e)),
                ));
            }
        };

        if !has_platform_info(&target) {
            return Err(platform_function_exception(invalid_platform_exception(
                platform_label,
            )));
        }

        let configured_target = match env.get_value_or_throw_with(
            Self::configured_target_dep(platform_label),
            catch_configured_value_creation_exception,
        )? {
            Ok(Some(v)) => v.get_configured_target().clone(),
            Ok(None) => return Ok(None),
            Err(e) => {
                return Err(platform_function_exception(
                    invalid_platform_exception_with_cause(platform_label, e),
                ));
            }
        };

        let Some(platform_info) = platform_provider_utils::platform(Some(&configured_target))
        else {
            return Err(platform_function_exception(invalid_platform_exception(
                &configured_target.get_label(),
            )));
        };
        Ok(Some(platform_info))
    }
}

/// The PLATFORM value the configuration code reads (see `blaze_build_config::platform`).
fn platform_value_of(
    platform_info: &Arc<PlatformInfo>,
    parsed_flags: Option<Arc<blaze_build_config::parsed_flags::ParsedFlagsValue>>,
) -> PlatformValue {
    // PORT: the seam's PlatformValue (blaze_build_config cannot name PlatformInfo) keeps
    // `constraints().get(setting)` for every setting the platform (or a parent) sets. Java's
    // `ConstraintCollection.get` also answers with a setting's default_constraint_value for a
    // setting the platform does not set; the one reader of the seam,
    // BuildConfigurationFunction.getPlatformCpu, asks for
    // ConstraintConstants.CPU_CONSTRAINT_SETTING, which is created without a default, so the
    // answer is the same.
    let mut constraints = BTreeMap::new();
    for setting in platform_info.constraints().all_constraint_settings() {
        if let Some(value) = platform_info.constraints().get(&setting) {
            constraints.insert(setting.label().clone(), value.label().clone());
        }
    }
    PlatformValue {
        label: platform_info.label().clone(),
        constraints,
        has_parsed_flags: parsed_flags.is_some(),
        parsed_flags,
        platform_info: Some(blaze_build_config::platform::OpaquePlatformInfo::new(
            platform_info.clone(),
        )),
    }
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformFunction.java:128 PlatformFunction.PlatformFunctionException
fn platform_function_exception(
    cause: blaze_analysis::toolchains::ToolchainException,
) -> ComputeError {
    SkyFunctionException::new(cause, Transience::Persistent).into()
}

impl PlatformValueProvider for PlatformFunction {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformFunction.java:57 PlatformFunction.compute
    fn compute(
        &self,
        key: &PlatformValueKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PlatformValue>, ComputeError> {
        let platform_label = &key.platform_label;
        let Some(platform_info) = Self::compute_platform_info(platform_label, env)? else {
            return Ok(None);
        };

        let platform_info = Arc::new(platform_info);
        if platform_info.flags().is_empty() {
            // Java: PlatformValue.noFlags(platformInfo)
            return Ok(Some(platform_value_of(&platform_info, None)));
        }

        let Some(repo_mapping_value) = env.get_value(RepositoryMappingKey::key(
            platform_label.get_repository().clone(),
        ))?
        else {
            return Ok(None);
        };

        let parsed_flags_key = ParsedFlagsKey::create_with_defaults(
            platform_info.flags().to_vec(),
            LabelPackageContext::of(
                platform_label.get_package_identifier(),
                repo_mapping_value
                    .repository_mapping()
                    .expect("NullPointerException: repositoryMapping()")
                    .clone(),
            ),
            // Include default values so that any flags explicitly reset to the default are kept.
            /* include_default_values= */
            true,
            key.flag_alias_mappings.clone(),
        );
        let Some(parsed_flags_value) = env.get_value(parsed_flags_key)? else {
            return Ok(None);
        };

        // Java: PlatformValue.withFlags(platformInfo, parsedFlagsValue)
        Ok(Some(platform_value_of(
            &platform_info,
            Some(parsed_flags_value),
        )))
    }

    /// `InvalidPlatformException` is a `ToolchainException` of kind `InvalidPlatform` (what
    /// `compute` throws through `platform_function_exception`).
    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:176 PlatformLookupUtil.InvalidPlatformException
    fn catch_invalid_platform_exception(&self, exception: &SkyError) -> Option<SkyError> {
        exception
            .downcast_ref::<blaze_analysis::toolchains::ToolchainException>()
            .filter(|t| {
                t.kind() == blaze_analysis::toolchains::ToolchainExceptionKind::InvalidPlatform
            })
            .map(|_| exception.clone())
    }
}

/// Shared handle for `ConfigurationRequest.platforms`.
pub fn platform_function(services: Arc<AnalysisServices>) -> Arc<dyn PlatformValueProvider> {
    Arc::new(PlatformFunction::new(services))
}
