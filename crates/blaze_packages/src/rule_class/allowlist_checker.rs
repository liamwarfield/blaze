//! Port of `packages/AllowlistChecker.java`.

/// Java `AllowlistChecker.LocationCheck`.
// Java: com/google/devtools/build/lib/packages/AllowlistChecker.java:45 AllowlistChecker.LocationCheck
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationCheck {
    /// Pass if the rule instance is in the allowlist.
    Instance,
    /// Pass if the rule definition is in the allowlist.
    Definition,
    /// Pass if either is.
    InstanceOrDefinition,
}

/// An allowlist check a rule class performs during analysis (Java `AllowlistChecker`).
// Java: com/google/devtools/build/lib/packages/AllowlistChecker.java:33 AllowlistChecker
// PORT: skipped, the AutoBuilder is a struct literal at its callers: AllowlistChecker.builder, AllowlistChecker.Builder.setAllowlistAttr, AllowlistChecker.Builder.setErrorMessage, AllowlistChecker.Builder.setAttributeSetTrigger, AllowlistChecker.Builder.setLocationCheck, AllowlistChecker.Builder.build
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AllowlistChecker {
    pub allowlist_attr: String,
    pub error_message: String,
    pub location_check: LocationCheck,
    pub attribute_set_trigger: Option<String>,
}
