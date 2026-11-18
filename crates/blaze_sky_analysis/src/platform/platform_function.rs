//! Port of `analysis/platform/PlatformFunction.java`: validates that a label is a platform, then
//! requests its configured target, extracts the `PlatformInfo` and parses any
//! `PlatformInfo.flags()`.
//!
//! The PLATFORM SkyFunction is registered by the configuration functions
//! (`blaze_build_config::skyframe::register_configuration_functions`) through the
//! [`PlatformValueProvider`] seam; [`PlatformFunction`] is its implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_build_config::platform::{PlatformValue, PlatformValueKey, PlatformValueProvider};
use blaze_cmdline::Label;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::{ComputeError, Environment, SkyFunctionException, Transience};

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
        // Use the empty configuration, since platforms are not configurable.
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
        // until after verification. See https://github.com/bazelbuild/bazel/issues/18215.

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
fn platform_value_of(platform_info: &PlatformInfo, has_parsed_flags: bool) -> PlatformValue {
    // PORT: the seam's PlatformValue keeps `constraints().get(setting)` for every setting the
    // platform (or a parent) sets; defaults of settings the platform does not set are not
    // representable there.
    let mut constraints = BTreeMap::new();
    for setting in platform_info.constraints().all_constraint_settings() {
        if let Some(value) = platform_info.constraints().get(&setting) {
            constraints.insert(setting.label().clone(), value.label().clone());
        }
    }
    PlatformValue {
        label: platform_info.label().clone(),
        constraints,
        has_parsed_flags,
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

        if platform_info.flags().is_empty() {
            return Ok(Some(platform_value_of(&platform_info, false)));
        }

        let Some(_repo_mapping_value) = env.get_value(RepositoryMappingKey::key(
            platform_label.get_repository().clone(),
        ))?
        else {
            return Ok(None);
        };

        // Java: ParsedFlagsValue.Key.create(platformInfo.flags(), PackageContext.of(pkgId,
        // repoMapping), includeDefaultValues=true, params.flagAliasMappings()), then
        // PlatformValue.withFlags.
        blaze_util::unported!(
            "ParsedFlagsValue.Key.create",
            "platform flags of {}",
            platform_label
        )
    }
}

/// Shared handle for `ConfigurationRequest.platforms`.
pub fn platform_function(services: Arc<AnalysisServices>) -> Arc<dyn PlatformValueProvider> {
    Arc::new(PlatformFunction::new(services))
}
