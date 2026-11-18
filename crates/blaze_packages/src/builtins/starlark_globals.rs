//! The fixed top-level symbols of each Starlark dialect (Java `StarlarkGlobals`, implemented by
//! `StarlarkGlobalsImpl`).
//!
//! The symbols themselves are ported by several agents (blaze_starlark: depset, json, struct,
//! Label; B: rule(), attr, provider(), ...; C: select(), BUILD globals, native module functions),
//! so [`StarlarkGlobalsFns`] holds their registration functions; `blaze_sky_loading` assembles
//! Bazel's set. Each symbol is created once, so every environment shares the same objects (as
//! Java's singletons are shared).
//!
//! Java: com/google/devtools/build/lib/packages/StarlarkGlobals.java,
//! com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java.

use blaze_starlark::env::NativeFn;

use crate::builtins::env::{StarlarkEnv, env_from_registration};

/// Registration functions for the fixed symbols of each dialect.
#[derive(Clone, Default)]
pub struct StarlarkGlobalsFns {
    /// Java `addCommonUtilToplevels` + `SelectLibrary`: depset, json, proto, select.
    pub util_toplevels: Vec<NativeFn>,
    /// Java `StarlarkNativeModule.BINDINGS_FOR_BUILD_FILES`.
    pub build_file_toplevels_shared_with_native: Vec<NativeFn>,
    /// Java `BuildGlobals` (BUILD-only, not in `native`).
    pub build_globals: Vec<NativeFn>,
    /// Java `BazelBuildApiGlobals`, `StarlarkRuleClassFunctions`, `attr`, `struct`, the fixed
    /// providers (and `Label`).
    pub bzl_globals: Vec<NativeFn>,
    /// Java `getSclToplevels` besides `visibility` (taken from `bzl_globals`): `struct`.
    pub scl_toplevels: Vec<NativeFn>,
}

fn env_of(fns: &[NativeFn]) -> StarlarkEnv {
    let mut env = StarlarkEnv::new();
    for f in fns {
        for (k, v) in env_from_registration(f) {
            env.insert(k, v);
        }
    }
    env
}

fn union(a: &StarlarkEnv, b: &StarlarkEnv) -> StarlarkEnv {
    let mut env = a.clone();
    for (k, v) in b {
        env.insert(k.clone(), v.clone());
    }
    env
}

/// The fixed symbols of each dialect.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:43 StarlarkGlobalsImpl
#[derive(Clone)]
pub struct StarlarkGlobals {
    util_toplevels: StarlarkEnv,
    fixed_build_file_toplevels_shared_with_native: StarlarkEnv,
    fixed_build_file_toplevels_not_in_native: StarlarkEnv,
    fixed_bzl_toplevels: StarlarkEnv,
    scl_toplevels: StarlarkEnv,
}

impl StarlarkGlobals {
    pub fn new(fns: &StarlarkGlobalsFns) -> StarlarkGlobals {
        let util_toplevels = env_of(&fns.util_toplevels);
        let bzl = env_of(&fns.bzl_globals);
        let fixed_bzl_toplevels = union(&util_toplevels, &bzl);
        // Java getSclToplevels: TODO(bazel-team): We only want the visibility() symbol from
        // BazelBuildApiGlobals, nothing else.
        let mut scl_toplevels = StarlarkEnv::new();
        if let Some(visibility) = bzl.get("visibility") {
            scl_toplevels.insert("visibility".to_owned(), visibility.clone());
        }
        let scl = env_of(&fns.scl_toplevels);
        let scl_toplevels = union(&scl_toplevels, &scl);
        StarlarkGlobals {
            fixed_build_file_toplevels_shared_with_native: env_of(
                &fns.build_file_toplevels_shared_with_native,
            ),
            fixed_build_file_toplevels_not_in_native: union(
                &util_toplevels,
                &env_of(&fns.build_globals),
            ),
            util_toplevels,
            fixed_bzl_toplevels,
            scl_toplevels,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:59 StarlarkGlobalsImpl.getUtilToplevels
    pub fn get_util_toplevels(&self) -> StarlarkEnv {
        self.util_toplevels.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:77 StarlarkGlobalsImpl.getFixedBuildFileToplevelsSharedWithNative
    pub fn get_fixed_build_file_toplevels_shared_with_native(&self) -> StarlarkEnv {
        self.fixed_build_file_toplevels_shared_with_native.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:82 StarlarkGlobalsImpl.getFixedBuildFileToplevelsNotInNative
    pub fn get_fixed_build_file_toplevels_not_in_native(&self) -> StarlarkEnv {
        self.fixed_build_file_toplevels_not_in_native.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:92 StarlarkGlobalsImpl.getFixedBzlToplevels
    pub fn get_fixed_bzl_toplevels(&self) -> StarlarkEnv {
        self.fixed_bzl_toplevels.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:111 StarlarkGlobalsImpl.getSclToplevels
    pub fn get_scl_toplevels(&self) -> StarlarkEnv {
        self.scl_toplevels.clone()
    }
}
