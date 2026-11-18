//! Port of `skyframe/toolchains/PlatformLookupUtil.java`: helper class that looks up
//! `PlatformInfo` data.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::toolchains::toolchain_exception::no_such_thing_java_class_name;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_packages::package::Package;
use blaze_packages::starlark_defs::platform_common::PLATFORM_INFO;
use blaze_packages::target::Target;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{Environment, SkyError, SkyKey};
use indexmap::IndexSet;

use crate::toolchains::lookup_error::{
    ConfiguredTargetLookupError, ToolchainLookupError, catch_configured_target_error,
};

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:49 PlatformLookupUtil.getPlatformInfo
pub fn get_platform_info(
    platform_keys: &[ConfiguredTargetKey],
    env: &mut Environment<'_>,
) -> Result<Option<HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>>, ToolchainLookupError> {
    validate_platform_keys(platform_keys, env)?;
    if env.values_missing() {
        return Ok(None);
    }

    let sky_keys: Vec<SkyKey> = platform_keys.iter().cloned().map(SkyKey::new).collect();
    let values = env.get_values_and_exceptions(&sky_keys)?;
    let values_missing = env.values_missing();
    let mut platforms: HashMap<ConfiguredTargetKey, Arc<PlatformInfo>> = HashMap::new();
    for key in platform_keys {
        let platform_info = find_platform_info(key, &values)?;
        if !values_missing && let Some(p) = platform_info {
            platforms.insert(key.clone(), p);
        }
    }
    if values_missing {
        return Ok(None);
    }

    Ok(Some(platforms))
}

/// Validate that all keys are for actual platform targets.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:74 PlatformLookupUtil.validatePlatformKeys
fn validate_platform_keys(
    platform_keys: &[ConfiguredTargetKey],
    env: &mut Environment<'_>,
) -> Result<(), ToolchainLookupError> {
    // Load the packages. This should already be in Skyframe and thus not require a restart.
    let package_keys: IndexSet<PackageIdentifier> = platform_keys
        .iter()
        .map(|k| k.get_label().get_package_identifier().clone())
        .collect();

    let sky_keys: Vec<SkyKey> = package_keys
        .iter()
        .map(|p| SkyKey::new(PackageKey(p.clone())))
        .collect();
    let values = env.get_values_and_exceptions(&sky_keys)?;
    let values_missing = env.values_missing();
    let mut packages: HashMap<PackageIdentifier, Arc<Package>> = HashMap::new();
    for package_key in &package_keys {
        match values.get_or_throw_typed::<PackageKey, NoSuchPackageException>(&PackageKey(
            package_key.clone(),
        )) {
            Ok(Some(package_value)) => {
                if !values_missing {
                    packages.insert(package_key.clone(), package_value.get_package().clone());
                }
            }
            Ok(None) => {}
            Err(e) => {
                return Err(invalid_platform_exception_no_such_thing(
                    NoSuchThingException::Package(e),
                )
                .into());
            }
        }
    }
    if env.values_missing() {
        // PORT: skipped, Google-internal: `if (valuesMissing != env.valuesMissing())
        // BugReport.sendBugReport(new IllegalStateException("Some value from " + packageKeys +
        // " was missing, this should never happen"))` only logs the report in a released
        // binary; Java returns here in both cases.
        return Ok(());
    }

    // Now check each platform.
    for platform_key in platform_keys {
        let platform_label = platform_key.get_label();
        let pkg = &packages[platform_label.get_package_identifier()];
        match pkg.get_target(platform_label.get_name()) {
            Ok(target) => {
                if !has_platform_info(target) {
                    // validation failure
                    return Err(invalid_platform_exception(platform_label).into());
                }
            }
            Err(e) => {
                return Err(invalid_platform_exception_no_such_thing(
                    NoSuchThingException::Target(e),
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Returns the [`PlatformInfo`] provider from the configured target in the lookup result, or
/// `None` if the configured target is not present. If the configured target does not have a
/// [`PlatformInfo`] provider, an `InvalidPlatformException` is thrown.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:130 PlatformLookupUtil.findPlatformInfo
fn find_platform_info(
    key: &ConfiguredTargetKey,
    values: &blaze_skyframe::SkyframeLookupResult,
) -> Result<Option<Arc<PlatformInfo>>, ToolchainException> {
    match values.get_or_throw_with(key, catch_configured_target_error) {
        Ok(None) => Ok(None),
        Ok(Some(ctv)) => {
            let configured_target = ctv.get_configured_target();
            let Some(platform_info) = platform_provider_utils::platform(Some(configured_target))
            else {
                return Err(invalid_platform_exception(&configured_target.get_label()));
            };
            Ok(Some(Arc::new(platform_info)))
        }
        Err(ConfiguredTargetLookupError::ConfiguredValueCreation(e)) => {
            Err(invalid_platform_exception_with_cause(key.get_label(), e))
        }
        Err(ConfiguredTargetLookupError::NoSuchThing(e)) => {
            Err(invalid_platform_exception_no_such_thing(e))
        }
        Err(ConfiguredTargetLookupError::ActionConflict(e)) => {
            Err(invalid_platform_exception_with_cause(key.get_label(), e))
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:160 PlatformLookupUtil.hasPlatformInfo
pub fn has_platform_info(target: &Target) -> bool {
    let Some(rule) = target.get_associated_rule() else {
        return false;
    };
    // If the rule uses toolchain resolution, it can't be used as a target or exec platform.
    if rule.use_toolchain_resolution() {
        return false;
    }

    rule.get_rule_class_object()
        .get_advertised_providers()
        .advertises(&PLATFORM_INFO.id())
}

/// Java `FilteringPolicy` shape of [`has_platform_info`] for `TargetPatternUtil`.
pub fn has_platform_info_filter(target: &Target, explicit: bool) -> bool {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:70 RegisteredExecutionPlatformsFunction.HAS_PLATFORM_INFO
    explicit || has_platform_info(target)
}

const INVALID_PLATFORM_DEFAULT_ERROR: &str = "does not provide PlatformInfo";

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:201 PlatformLookupUtil.InvalidPlatformException.formatError
fn format_error(label: &Label, error: &str) -> String {
    format!("Target {label} was referenced as a platform, but {error}")
}

/// Java `new InvalidPlatformException(Label)`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:179 PlatformLookupUtil.InvalidPlatformException.<init>
pub fn invalid_platform_exception(label: &Label) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::InvalidPlatform,
        format_error(label, INVALID_PLATFORM_DEFAULT_ERROR),
    )
}

/// Java `new InvalidPlatformException(Label, ConfiguredValueCreationException)` (also the
/// `ActionConflictException` overload).
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:183 PlatformLookupUtil.InvalidPlatformException.<init>
pub fn invalid_platform_exception_with_cause(label: &Label, cause: SkyError) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidPlatform,
        format_error(label, INVALID_PLATFORM_DEFAULT_ERROR),
        cause,
    )
}

/// Java `new InvalidPlatformException(NoSuchThingException)`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:187 PlatformLookupUtil.InvalidPlatformException.<init>
pub fn invalid_platform_exception_no_such_thing(e: NoSuchThingException) -> ToolchainException {
    // Just propagate the inner exception, because it's directly actionable.
    let class = no_such_thing_java_class_name(&e);
    ToolchainException::from_cause(ToolchainExceptionKind::InvalidPlatform, class, Arc::new(e))
}
