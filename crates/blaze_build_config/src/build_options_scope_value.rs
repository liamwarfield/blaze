//! Port of `skyframe/BuildOptionsScopeValue.java`: the key and value of BUILD_OPTIONS_SCOPE.
//!
//! The function (`skyframe/BuildOptionsScopeFunction.java`) loads the packages of the flags, so
//! it is registered by `blaze_sky_analysis::build_options_scope_function`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName};
use indexmap::IndexMap;

use crate::build_options::{BuildOptions, Scope};

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:38 BuildOptionsScopeValue.Key
/// Key for [`BuildOptionsScopeValue`].
#[derive(Clone)]
pub struct BuildOptionsScopeKey {
    build_options: Arc<BuildOptions>,
    flags_with_incomplete_scope_info: Vec<Label>,
}

impl BuildOptionsScopeKey {
    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:48 BuildOptionsScopeValue.Key.create
    pub fn create(
        build_options: Arc<BuildOptions>,
        flags_with_incomplete_scope_info: Vec<Label>,
    ) -> Self {
        BuildOptionsScopeKey {
            build_options,
            flags_with_incomplete_scope_info,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:52 BuildOptionsScopeValue.Key.getBuildOptions
    pub fn get_build_options(&self) -> &Arc<BuildOptions> {
        &self.build_options
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:60 BuildOptionsScopeValue.Key.getFlagsWithIncompleteScopeInfo
    /// Returns the list of flags that are either project scoped or their scopes are not yet
    /// resolved.
    pub fn get_flags_with_incomplete_scope_info(&self) -> &[Label] {
        &self.flags_with_incomplete_scope_info
    }
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:75 BuildOptionsScopeValue.Key.equals
impl PartialEq for BuildOptionsScopeKey {
    fn eq(&self, other: &Self) -> bool {
        *self.build_options == *other.build_options
            && self.flags_with_incomplete_scope_info == other.flags_with_incomplete_scope_info
    }
}
impl Eq for BuildOptionsScopeKey {}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:88 BuildOptionsScopeValue.Key.hashCode
impl Hash for BuildOptionsScopeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.build_options.hash(state);
        self.flags_with_incomplete_scope_info.hash(state);
    }
}

impl fmt::Debug for BuildOptionsScopeKey {
    // PORT: the Java key has no toString (Object.toString prints the identity hash).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildOptionsScopeValue.Key{{buildOptions={}, flagsWithIncompleteScopeInfo=[{}]}}",
            self.build_options.checksum(),
            self.flags_with_incomplete_scope_info
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Key for BuildOptionsScopeKey {
    type Value = BuildOptionsScopeValue;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:185 SkyFunctions.BUILD_OPTIONS_SCOPE
    const FUNCTION: SkyFunctionName = SkyFunctionName("BUILD_OPTIONS_SCOPE");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:186 SkyFunctions.BUILD_OPTIONS_SCOPE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): Java's `BuildOptionsScopeValue` has no `equals`.
    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:29 BuildOptionsScopeValue
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:29 BuildOptionsScopeValue
/// SkyValue returned by `BuildOptionsScopeFunction`.
#[derive(Clone, Debug)]
pub struct BuildOptionsScopeValue {
    resolved_build_options_with_scope_types: Arc<BuildOptions>,
    scoped_flags: Vec<Label>,
    /// Java `LinkedHashMap<Label, Scope>`.
    fully_resolved_scopes: IndexMap<Label, Scope>,
}

impl BuildOptionsScopeValue {
    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:93 BuildOptionsScopeValue.create
    pub fn create(
        input_build_options: Arc<BuildOptions>,
        scoped_flags: Vec<Label>,
        fully_resolved_scopes: IndexMap<Label, Scope>,
    ) -> Self {
        BuildOptionsScopeValue {
            resolved_build_options_with_scope_types: input_build_options,
            scoped_flags,
            fully_resolved_scopes,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:114 BuildOptionsScopeValue.getResolvedBuildOptionsWithScopeTypes
    /// Returns the `BuildOptions` with the all starlark flags having their `Scope.ScopeType`
    /// resolved.
    pub fn get_resolved_build_options_with_scope_types(&self) -> &Arc<BuildOptions> {
        &self.resolved_build_options_with_scope_types
    }

    /// The flags that have a PROJECT.scl file (Java field `scopedFlags`).
    pub fn get_scoped_flags(&self) -> &[Label] {
        &self.scoped_flags
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeValue.java:122 BuildOptionsScopeValue.getFullyResolvedScopes
    /// Returns the map of `Label` of scoped flags to their `Scope` including both
    /// `Scope.ScopeType` and `Scope.ScopeDefinition`.
    pub fn get_fully_resolved_scopes(&self) -> &IndexMap<Label, Scope> {
        &self.fully_resolved_scopes
    }
}
