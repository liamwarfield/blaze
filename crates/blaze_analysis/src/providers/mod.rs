//! Native providers (agent R; crates/blaze_analysis/DESIGN.md "Infos are frozen Starlark
//! values"): `DefaultInfo`, `OutputGroupInfo`, `RunEnvironmentInfo`, `FileProvider`,
//! `FilesToRunProvider`, `RunfilesProvider`, `VisibilityProvider`,
//! `PackageSpecificationProvider`, `TransitiveVisibilityProvider`,
//! `RequiredConfigFragmentsProvider`, `AliasProvider` and the Starlark `runfiles` value.

pub mod alias_provider;
pub mod default_info;
pub mod file_provider;
pub mod materialized_deps_info;
pub mod output_group_info;
pub mod package_specification_provider;
pub mod required_config_fragments_provider;
pub mod run_environment_info;
pub mod starlark_runfiles;
pub mod visibility;

use blaze_packages::starlark_defs::builtin_providers::{
    BuiltinConstructor, BuiltinProvider, BuiltinProviderData, DEFAULT_INFO,
    MATERIALIZED_DEPS_INFO, OUTPUT_GROUP_INFO, RUN_ENVIRONMENT_INFO,
};
use starlark::environment::GlobalsBuilder;

/// Java `DefaultInfo.PROVIDER` with its constructor (analysis-capable; same identity as
/// `blaze_packages::starlark_defs::builtin_providers::default_info()`).
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:34 DefaultInfo.PROVIDER
pub const fn default_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: DEFAULT_INFO,
        constructor: BuiltinConstructor::Native(default_info::default_info_constructor),
    })
}

/// Java `OutputGroupInfo.STARLARK_CONSTRUCTOR` with its constructor.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:79 OutputGroupInfo.STARLARK_CONSTRUCTOR
pub const fn output_group_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: OUTPUT_GROUP_INFO,
        constructor: BuiltinConstructor::Native(output_group_info::output_group_info_constructor),
    })
}

/// Java `RunEnvironmentInfo.PROVIDER` with its constructor.
// Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:39 RunEnvironmentInfo.PROVIDER
pub const fn run_environment_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: RUN_ENVIRONMENT_INFO,
        constructor: BuiltinConstructor::Native(
            run_environment_info::run_environment_info_constructor,
        ),
    })
}

/// Java `MaterializedDepsInfo.PROVIDER` with its constructor.
// Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:31 MaterializedDepsInfo.PROVIDER
pub const fn materialized_deps_info_provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: MATERIALIZED_DEPS_INFO,
        constructor: BuiltinConstructor::Native(
            materialized_deps_info::materialized_deps_info_constructor,
        ),
    })
}

/// Defines the analysis-capable `DefaultInfo`, `OutputGroupInfo`, `RunEnvironmentInfo` and
/// `MaterializedDepsInfo` toplevels (same `BuiltinProviderId`s as W3's, so infos created by either are found by
/// either symbol). The lead substitutes these for W3's entries.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:101 StarlarkGlobalsImpl.getFixedBzlToplevels
pub fn register_provider_toplevels(builder: &mut GlobalsBuilder) {
    builder.set("DefaultInfo", default_info_provider());
    builder.set("OutputGroupInfo", output_group_info_provider());
    builder.set("RunEnvironmentInfo", run_environment_info_provider());
    builder.set("MaterializedDepsInfo", materialized_deps_info_provider());
}
