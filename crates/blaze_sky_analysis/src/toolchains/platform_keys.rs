//! Port of `skyframe/toolchains/PlatformKeys.java`: details of platforms used during toolchain
//! resolution.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::{PlatformInfo, ToolchainTypeInfo};
use blaze_analysis::toolchains::ToolchainExceptionKind;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_build_config::transitions::EMPTY_OPTIONS;
use blaze_cmdline::Label;
use blaze_skyframe::Environment;
use indexmap::{IndexMap, IndexSet};

use crate::toolchains::constraint_value_lookup_util;
use crate::toolchains::lookup_error::{ToolchainLookupError, catch_toolchain_exception};
use crate::toolchains::platform_lookup_util;
use crate::toolchains::registered_execution_platforms::RegisteredExecutionPlatformsKey;
use crate::toolchains::toolchain_resolution_function::ToolchainType;

/// Java `Table<ConfiguredTargetKey, ToolchainTypeInfo, Label>` (Guava `HashBasedTable`, whose
/// iteration follows insertion order).
pub type ResolvedToolchainsTable =
    IndexMap<ConfiguredTargetKey, IndexMap<ToolchainTypeInfo, Label>>;

/// `BuildConfigurationKey.create(CommonOptions.EMPTY_OPTIONS)`.
pub fn empty_options_configuration_key() -> BuildConfigurationKey {
    BuildConfigurationKey::create(Arc::new(EMPTY_OPTIONS.clone()))
}

/// Details of platforms used during toolchain resolution.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:45 PlatformKeys
#[derive(Clone, Debug)]
pub struct PlatformKeys {
    target_platform_key: ConfiguredTargetKey,
    execution_platform_keys: Vec<ConfiguredTargetKey>,
    platform_infos: HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>,
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:50 PlatformKeys.Builder
struct Builder<'e, 'a> {
    // Input data.
    environment: &'e mut Environment<'a>,
    configuration_key: BuildConfigurationKey,

    // Internal state used during loading.
    host_platform_label: Label,
    target_platform_label: Label,
    host_platform_key: Option<ConfiguredTargetKey>,
    execution_platform_keys: IndexSet<ConfiguredTargetKey>,
    platform_infos: HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>,
}

impl<'e, 'a> Builder<'e, 'a> {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:63 PlatformKeys.Builder.<init>
    fn new(
        environment: &'e mut Environment<'a>,
        configuration_key: BuildConfigurationKey,
        platform_configuration: &PlatformConfiguration,
    ) -> Self {
        Builder {
            environment,
            configuration_key,
            host_platform_label: platform_configuration.get_host_platform().clone(),
            target_platform_label: platform_configuration.get_target_platform().clone(),
            host_platform_key: None,
            execution_platform_keys: IndexSet::new(),
            platform_infos: HashMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:76 PlatformKeys.Builder.build
    fn build(
        mut self,
        exec_constraint_labels: &[Label],
    ) -> Result<Option<PlatformKeys>, ToolchainLookupError> {
        // Determine the execution platform keys.
        if self.find_execution_platform_keys()?.is_none() {
            return Ok(None);
        }

        // Load the PlatformInfo for all relevant platforms (target, host, exec).
        if self.load_platform_info()?.is_none() {
            return Ok(None);
        }

        // Update platform keys to resolve possible aliases.
        let resolved_target_platform_key =
            self.update_configured_target_key(&self.target_platform_label.clone());
        let resolved_host_platform_key =
            self.update_configured_target_key(&self.host_platform_label.clone());
        if self.host_platform_key != resolved_host_platform_key {
            // Replace the host platform key in the execution platform keys with the updated
            // version.
            if let Some(host) = &self.host_platform_key {
                self.execution_platform_keys.shift_remove(host);
            }
            let resolved = resolved_host_platform_key
                .clone()
                .expect("NullPointerException: host platform key");
            self.execution_platform_keys.insert(resolved);
            self.host_platform_key = resolved_host_platform_key;
        }

        // Ensure all platforms are present in platformInfos by original key and by
        // alias-resolved key, if they are different.
        let mut updated: Vec<(ConfiguredTargetKey, Arc<PlatformInfo>)> = Vec::new();
        for (original_key, info) in &self.platform_infos {
            let resolved_key = self.update_configured_target_key(original_key.get_label());
            if resolved_key.as_ref() != Some(original_key) {
                updated.push((
                    resolved_key.expect("NullPointerException: resolved key"),
                    info.clone(),
                ));
            }
        }
        self.platform_infos.extend(updated);

        // Filter the execution platforms, based on the applied constraints (if any).
        let Some(execution_platform_keys) =
            self.filter_execution_platforms(exec_constraint_labels)?
        else {
            return Ok(None);
        };

        Ok(Some(PlatformKeys {
            target_platform_key: resolved_target_platform_key
                .expect("NullPointerException: target platform key"),
            execution_platform_keys,
            platform_infos: self.platform_infos,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:120 PlatformKeys.Builder.findExecutionPlatformKeys
    fn find_execution_platform_keys(&mut self) -> Result<Option<()>, ToolchainLookupError> {
        // Find the registered execution platforms.
        let registered_execution_platforms = match self.environment.get_value_or_throw_with(
            // PORT: the debug printer is not ported; debugEnabled() is false here.
            RegisteredExecutionPlatformsKey::key(self.configuration_key.clone(), false),
            catch_toolchain_exception(&[
                ToolchainExceptionKind::InvalidPlatform,
                ToolchainExceptionKind::InvalidExecutionPlatformLabel,
            ]),
        )? {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        // If debugging, describe rejected execution platforms.
        // PORT: rejectedPlatforms is only read by the debug printer.

        // A given execution platform may be registered multiple times, but only the earliest
        // registration is meaningful in practice. To avoid unnecessary reprocessing a
        // LinkedHashSet is used to de-duplicate while preserving order.
        self.execution_platform_keys = IndexSet::new();
        self.execution_platform_keys.extend(
            registered_execution_platforms
                .registered_execution_platform_keys
                .iter()
                .cloned(),
        );
        let host_platform_key = ConfiguredTargetKey::builder()
            .set_label(self.host_platform_label.clone())
            .set_configuration_key(Some(empty_options_configuration_key()))
            .build();
        self.execution_platform_keys
            .insert(host_platform_key.clone());
        self.host_platform_key = Some(host_platform_key);
        Ok(Some(()))
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:155 PlatformKeys.Builder.loadPlatformInfo
    fn load_platform_info(&mut self) -> Result<Option<()>, ToolchainLookupError> {
        let mut platform_keys: Vec<ConfiguredTargetKey> = Vec::new();
        platform_keys.push(
            ConfiguredTargetKey::builder()
                .set_label(self.target_platform_label.clone())
                .set_configuration_key(Some(empty_options_configuration_key()))
                .build(),
        );
        platform_keys.extend(self.execution_platform_keys.iter().cloned());

        let platform_infos =
            platform_lookup_util::get_platform_info(&platform_keys, self.environment)?;
        if self.environment.values_missing() {
            return Ok(None);
        }
        self.platform_infos = platform_infos.expect("platform infos");
        Ok(Some(()))
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:177 PlatformKeys.Builder.updateConfiguredTargetKey
    fn update_configured_target_key(&self, platform_label: &Label) -> Option<ConfiguredTargetKey> {
        let platform_info = self
            .platform_infos
            .iter()
            .find(|(k, _)| k.get_label() == platform_label)
            .map(|(_, v)| v)?;
        Some(
            ConfiguredTargetKey::builder()
                .set_label(platform_info.label().clone())
                .set_configuration_key(Some(empty_options_configuration_key()))
                .build(),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:194 PlatformKeys.Builder.filterExecutionPlatforms
    fn filter_execution_platforms(
        &mut self,
        exec_constraint_labels: &[Label],
    ) -> Result<Option<Vec<ConfiguredTargetKey>>, ToolchainLookupError> {
        // Short circuit if not needed.
        if exec_constraint_labels.is_empty() {
            return Ok(Some(self.execution_platform_keys.iter().cloned().collect()));
        }

        // Filter out execution platforms that don't satisfy the extra constraints.
        let exec_constraint_keys: Vec<ConfiguredTargetKey> = exec_constraint_labels
            .iter()
            .map(|label| {
                ConfiguredTargetKey::builder()
                    .set_label(label.clone())
                    .set_configuration_key(Some(empty_options_configuration_key()))
                    .build()
            })
            .collect();

        let Some(constraints) = constraint_value_lookup_util::get_constraint_value_info(
            &exec_constraint_keys,
            self.environment,
        )?
        else {
            return Ok(None);
        };

        Ok(Some(
            self.execution_platform_keys
                .iter()
                .filter(|key| {
                    let platform_info = self
                        .platform_infos
                        .get(*key)
                        .expect("NullPointerException: platform info");
                    filter_platform(platform_info, &constraints)
                })
                .cloned()
                .collect(),
        ))
    }
}

/// Returns `true` if the given platform has all of the constraints.
// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:229 PlatformKeys.Builder.filterPlatform
fn filter_platform(
    platform_info: &PlatformInfo,
    constraints: &[blaze_analysis::platform::ConstraintValueInfo],
) -> bool {
    let missing_constraints = platform_info.constraints().find_missing(constraints);
    // PORT: debugPrinter.reportRemovedExecutionPlatform is a no-op without debugging.
    missing_constraints.is_empty()
}

impl PlatformKeys {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:239 PlatformKeys.load
    pub fn load(
        environment: &mut Environment<'_>,
        configuration_key: BuildConfigurationKey,
        platform_configuration: &PlatformConfiguration,
        exec_constraint_labels: &[Label],
    ) -> Result<Option<PlatformKeys>, ToolchainLookupError> {
        Builder::new(environment, configuration_key, platform_configuration)
            .build(exec_constraint_labels)
    }

    pub fn target_platform_key(&self) -> &ConfiguredTargetKey {
        &self.target_platform_key
    }

    pub fn execution_platform_keys(&self) -> &[ConfiguredTargetKey] {
        &self.execution_platform_keys
    }

    pub fn platform_infos(&self) -> &HashMap<ConfiguredTargetKey, Arc<PlatformInfo>> {
        &self.platform_infos
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:256 PlatformKeys.find
    pub fn find(&self, platform_label: &Label) -> Option<ConfiguredTargetKey> {
        if platform_label == self.target_platform_key.get_label() {
            return Some(self.target_platform_key.clone());
        }

        for configured_target_key in &self.execution_platform_keys {
            if platform_label == configured_target_key.get_label() {
                return Some(configured_target_key.clone());
            }
        }

        None
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:271 PlatformKeys.targetPlatformInfo
    pub fn target_platform_info(&self) -> Option<&Arc<PlatformInfo>> {
        self.platform_info(&self.target_platform_key)
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:276 PlatformKeys.platformInfo
    pub fn platform_info(
        &self,
        configured_target_key: &ConfiguredTargetKey,
    ) -> Option<&Arc<PlatformInfo>> {
        self.platform_infos.get(configured_target_key)
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformKeys.java:280 PlatformKeys.isPlatformSuitable
    pub fn is_platform_suitable(
        &self,
        execution_platform_key: &ConfiguredTargetKey,
        toolchain_types: &[ToolchainType],
        resolved_toolchains: &ResolvedToolchainsTable,
        check_allowed_toolchain_types: bool,
    ) -> bool {
        let execution_platform_info = self
            .platform_info(execution_platform_key)
            .expect("NullPointerException: execution platform info");
        if check_allowed_toolchain_types
            && execution_platform_info.check_toolchain_types()
            && toolchain_types.is_empty()
        {
            // This can't be suitable.
            return false;
        } else if toolchain_types.is_empty() {
            // Since there aren't any toolchains, we should be able to use any execution platform
            // that has made it this far.
            return true;
        }

        // Determine whether all mandatory toolchains are present.
        let row = resolved_toolchains.get(execution_platform_key);
        toolchain_types
            .iter()
            .filter(|t| t.mandatory())
            .map(|t| &t.toolchain_type_info)
            .all(|info| row.is_some_and(|r| r.contains_key(info)))
    }
}
