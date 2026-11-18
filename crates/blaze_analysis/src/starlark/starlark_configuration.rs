//! The Starlark `configuration` value (`ctx.configuration`): the Starlark API of
//! `analysis/config/BuildConfigurationValue.java` (`starlarkbuildapi/BuildConfigurationApi.java`)
//! over `blaze_build_config::build_configuration_value::BuildConfigurationValue`.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::actions::path_mapper::FileRoot;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFileRoot;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::repository_name::MAIN;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use super::util::fail_if_called_outside_default_allowlist;
use crate::rule::artifact_roots;

/// A `BuildConfigurationValue` as a Starlark value (type `configuration`).
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:89 BuildConfigurationValue
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkConfiguration(#[allocative(skip)] pub Arc<BuildConfigurationValue>);

starlark_simple_value!(StarlarkConfiguration);

/// Java's default `StarlarkValue.repr` (BuildConfigurationValue does not override it).
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
const CONFIGURATION_REPR: &str =
    "<unknown object com.google.devtools.build.lib.analysis.config.BuildConfigurationValue>";

impl fmt::Display for StarlarkConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(CONFIGURATION_REPR)
    }
}

/// The configuration's `CoreOptions` (Java `BuildConfigurationValue.options`).
pub(crate) fn core_options(configuration: &BuildConfigurationValue) -> &CoreOptions {
    configuration
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions")
}

/// Java `BuildConfigurationValue.isCodeCoverageEnabled()`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:731 BuildConfigurationValue.isCodeCoverageEnabled
pub(crate) fn is_code_coverage_enabled(configuration: &BuildConfigurationValue) -> bool {
    core_options(configuration).collect_code_coverage
}

/// Java `BuildConfigurationValue.allowUnresolvedSymlinks()`.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java BuildConfigurationValue.allowUnresolvedSymlinks
pub(crate) fn allow_unresolved_symlinks(configuration: &BuildConfigurationValue) -> bool {
    core_options(configuration).allow_unresolved_symlinks
}

#[starlark_value(type = "configuration")]
impl<'v> StarlarkValue<'v> for StarlarkConfiguration {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("configuration", configuration_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(CONFIGURATION_REPR);
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:341 BuildConfigurationValue.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkConfiguration>()
            .is_some_and(|o| *o.0 == *self.0))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java BuildConfigurationValue.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.checksum().hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:398 BuildConfigurationValue.getBinDir
    #[starlark(attribute)]
    fn bin_dir(this: &StarlarkConfiguration) -> starlark::Result<StarlarkFileRoot> {
        let root = this.0.get_bin_directory(&MAIN);
        Ok(StarlarkFileRoot(FileRoot::Artifact(
            artifact_roots::to_actions_root(&root),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:432 BuildConfigurationValue.getGenfilesDir
    #[starlark(attribute)]
    fn genfiles_dir(this: &StarlarkConfiguration) -> starlark::Result<StarlarkFileRoot> {
        let root = this.0.get_genfiles_directory(&MAIN);
        Ok(StarlarkFileRoot(FileRoot::Artifact(
            artifact_roots::to_actions_root(&root),
        )))
    }

    /// Returns the separator for PATH environment variable, which is ':' on Unix.
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:483 BuildConfigurationValue.getHostPathSeparator
    #[starlark(attribute)]
    fn host_path_separator(this: &StarlarkConfiguration) -> starlark::Result<String> {
        Ok(this.0.get_host_path_separator().to_owned())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:556 BuildConfigurationValue.getLocalShellEnvironment
    #[starlark(attribute)]
    fn default_shell_env<'v>(
        this: &StarlarkConfiguration,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let env = this.0.get_action_environment().get_fixed_env();
        Ok(heap.alloc(AllocDict(env.iter().map(|(k, v)| {
            (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value())
        }))))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:710 BuildConfigurationValue.getTestEnv
    #[starlark(attribute)]
    fn test_env<'v>(this: &StarlarkConfiguration, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let env = this.0.get_test_env();
        Ok(heap.alloc(AllocDict(env.iter().map(|(k, v)| {
            (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value())
        }))))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:731 BuildConfigurationValue.isCodeCoverageEnabled
    #[starlark(attribute)]
    fn coverage_enabled(this: &StarlarkConfiguration) -> starlark::Result<bool> {
        Ok(is_code_coverage_enabled(&this.0))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:736 BuildConfigurationValue.getShortId
    #[starlark(attribute)]
    fn short_id(this: &StarlarkConfiguration) -> starlark::Result<String> {
        Ok(this.0.short_id())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:678 BuildConfigurationValue.stampBinariesForStarlark
    fn stamp_binaries<'v>(
        this: &StarlarkConfiguration,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(this.0.stamp_binaries())
    }

    /// Returns true when building in the tool (exec) configuration.
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:756 BuildConfigurationValue.isToolConfiguration
    fn is_tool_configuration(this: &StarlarkConfiguration) -> starlark::Result<bool> {
        Ok(this.0.is_tool_configuration())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:451 BuildConfigurationValue.hasSeparateGenfilesDirectoryForStarlark
    fn has_separate_genfiles_directory<'v>(
        this: &StarlarkConfiguration,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(this.0.has_separate_genfiles_directory())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:539 BuildConfigurationValue.isSiblingRepositoryLayoutForStarlark
    fn is_sibling_repository_layout<'v>(
        this: &StarlarkConfiguration,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(this.0.sibling_repository_layout())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:889 BuildConfigurationValue.runfilesEnabledForStarlark
    fn runfiles_enabled<'v>(
        this: &StarlarkConfiguration,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(this.0.runfiles_enabled())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:966 BuildConfigurationValue.getDisabledFeatures
    fn disabled_features<'v>(
        this: &StarlarkConfiguration,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        fail_if_called_outside_default_allowlist(eval)?;
        // Java: StarlarkSet.immutableCopyOf(getDefaultFeatures().off())
        let _ = this;
        blaze_util::unported!("StarlarkSet.immutableCopyOf")
    }
}
