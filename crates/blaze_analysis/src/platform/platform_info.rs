//! Port of `analysis/platform/PlatformInfo.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::platform_common::PLATFORM_INFO;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StructureMarker;
use blaze_util::Fingerprint;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::collections::Hashed;
use starlark::eval::Evaluator;
use starlark::values::ValueLike;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::info::InfoProviderKey;
use crate::platform::constraint_collection::{
    gated_get_attr, gated_has_attr,
    ConstraintCollection, ConstraintCollectionBuilder, DuplicateConstraintException,
};
use crate::platform::constraint_value_info::ConstraintValueInfo;
use crate::platform::platform_constants;
use crate::platform::platform_properties::{PlatformProperties, PlatformPropertiesBuilder};

/// Provider for a platform, which is a group of constraints and values.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:41 PlatformInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PlatformInfo {
    /// PORT: `None` where Java's builder was never given a label (tests only).
    #[allocative(skip)]
    label: Option<Label>,
    #[allocative(skip)]
    constraints: ConstraintCollection,
    #[allocative(skip)]
    remote_execution_properties: String,
    /// execProperties will deprecate and replace remoteExecutionProperties
    #[allocative(skip)]
    exec_properties: PlatformProperties,
    #[allocative(skip)]
    flags: Vec<String>,
    #[allocative(skip)]
    required_settings: Vec<ConfigMatchingProvider>,
    #[allocative(skip)]
    check_toolchain_types: bool,
    #[allocative(skip)]
    allowed_toolchain_types: Vec<Label>,
    #[allocative(skip)]
    missing_toolchain_error_message: Option<String>,
}

starlark_simple_value!(PlatformInfo);

/// Empty [`PlatformInfo`] instance for invalid or empty (e.g. builtin) actions. See also
/// src/main/starlark/builtins_bzl/platforms/BUILD#empty
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:58 PlatformInfo.EMPTY_PLATFORM_INFO
pub static EMPTY_PLATFORM_INFO: LazyLock<PlatformInfo> = LazyLock::new(|| {
    let mut builder = PlatformInfo::builder();
    builder.set_label(platform_constants::internal_platform());
    builder
        .build()
        .unwrap_or_else(|e| panic!("VerifyException: {e}"))
});

impl PlatformInfo {
    /// The literal key that will be used to copy the remote execution properties from the parent
    /// [`PlatformInfo`] into a new [`PlatformInfo`]'s remote execution properties.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:49 PlatformInfo.PARENT_REMOTE_EXECUTION_KEY
    pub const PARENT_REMOTE_EXECUTION_KEY: &'static str = "{PARENT_REMOTE_EXECUTION_PROPERTIES}";

    /// Name used in Starlark for accessing this provider.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:52 PlatformInfo.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "PlatformInfo";

    /// Java `PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        PLATFORM_INFO.key()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:121 PlatformInfo.label
    pub fn label(&self) -> &Label {
        self.label
            .as_ref()
            .expect("NullPointerException: PlatformInfo without a label")
    }

    /// Java `label()` where it may be null (`String.valueOf`).
    pub fn label_or_null(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:126 PlatformInfo.constraints
    pub fn constraints(&self) -> &ConstraintCollection {
        &self.constraints
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:130 PlatformInfo.remoteExecutionProperties
    pub fn remote_execution_properties(&self) -> &str {
        &self.remote_execution_properties
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:134 PlatformInfo.execProperties
    pub fn exec_properties(&self) -> &IndexMap<String, String> {
        self.exec_properties.properties()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:138 PlatformInfo.flags
    pub fn flags(&self) -> &[String] {
        &self.flags
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:142 PlatformInfo.requiredSettings
    pub fn required_settings(&self) -> &[ConfigMatchingProvider] {
        &self.required_settings
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:146 PlatformInfo.checkToolchainTypes
    pub fn check_toolchain_types(&self) -> bool {
        self.check_toolchain_types
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:150 PlatformInfo.allowedToolchainTypes
    pub fn allowed_toolchain_types(&self) -> &[Label] {
        &self.allowed_toolchain_types
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:155 PlatformInfo.getMissingToolchainErrorMessage
    pub fn get_missing_toolchain_error_message(&self) -> Option<&str> {
        self.missing_toolchain_error_message.as_deref()
    }

    /// Add this platform to the given fingerprint.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:165 PlatformInfo.addTo
    pub fn add_to(&self, fp: &mut Fingerprint) {
        fp.add_string(&label_to_string(self.label.as_ref()));
        self.constraints.add_to_fingerprint(fp);
        fp.add_nullable_string(Some(&self.remote_execution_properties));
        fp.add_string_map(
            self.exec_properties
                .properties()
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        fp.add_strings(self.flags.iter());
        fp.add_strings(
            self.required_settings
                .iter()
                .map(|s| s.label().to_string())
                .collect::<Vec<_>>(),
        );
        fp.add_strings(
            self.allowed_toolchain_types
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>(),
        );
        fp.add_boolean(self.check_toolchain_types);
        fp.add_nullable_string(self.missing_toolchain_error_message.as_deref());
    }

    /// Returns a new builder for creating a fresh [`PlatformInfo`] instance.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:212 PlatformInfo.builder
    pub fn builder() -> PlatformInfoBuilder {
        PlatformInfoBuilder::default()
    }
}

/// The action layer's view of `PlatformInfo` (aquery `executionPlatform`, action keys).
impl blaze_actions::PlatformInfo for PlatformInfo {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:121 PlatformInfo.label
    fn label(&self) -> &Label {
        PlatformInfo::label(self)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:165 PlatformInfo.addTo
    fn add_to(&self, fp: &mut Fingerprint) {
        PlatformInfo::add_to(self, fp)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:134 PlatformInfo.execProperties
    fn exec_properties(&self) -> IndexMap<String, String> {
        PlatformInfo::exec_properties(self).clone()
    }
}

/// Java `String.valueOf(label)`.
fn label_to_string(label: Option<&Label>) -> String {
    match label {
        Some(l) => l.to_string(),
        None => "null".to_owned(),
    }
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:182 PlatformInfo.equals
impl PartialEq for PlatformInfo {
    fn eq(&self, that: &Self) -> bool {
        self.label == that.label
            && self.constraints == that.constraints
            && self.remote_execution_properties == that.remote_execution_properties
            && self.exec_properties == that.exec_properties
            && self.flags == that.flags
            && self.required_settings == that.required_settings
            && self.check_toolchain_types == that.check_toolchain_types
            && self.allowed_toolchain_types == that.allowed_toolchain_types
            && self.missing_toolchain_error_message == that.missing_toolchain_error_message
    }
}

impl Eq for PlatformInfo {}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:198 PlatformInfo.hashCode
// PORT: `requiredSettings` is left out of the hash (consistent with equals).
impl Hash for PlatformInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.label.hash(state);
        self.constraints.hash(state);
        self.remote_execution_properties.hash(state);
        for (k, v) in self.exec_properties.properties() {
            k.hash(state);
            v.hash(state);
        }
        self.flags.hash(state);
        self.check_toolchain_types.hash(state);
        self.allowed_toolchain_types.hash(state);
        self.missing_toolchain_error_message.hash(state);
    }
}

impl fmt::Display for PlatformInfo {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:160 PlatformInfo.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PlatformInfo({}, constraints={})",
            label_to_string(self.label.as_ref()),
            self.constraints
        )
    }
}

/// Builder class to facilitate creating valid [`PlatformInfo`] instances.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:217 PlatformInfo.Builder
#[derive(Clone, Debug, Default)]
pub struct PlatformInfoBuilder {
    parent: Option<PlatformInfo>,
    label: Option<Label>,
    constraints: ConstraintCollectionBuilder,
    remote_execution_properties: Option<String>,
    exec_properties_builder: PlatformPropertiesBuilder,
    flags: Vec<String>,
    required_settings: Vec<ConfigMatchingProvider>,
    check_toolchain_types: bool,
    allowed_toolchain_types: Vec<Label>,
    missing_toolchain_error_message: Option<String>,
}

impl PlatformInfoBuilder {
    /// Sets the parent [`PlatformInfo`] that this platform inherits from. Constraint values set
    /// directly on this instance will be kept, but any other constraint settings will be found
    /// from the parent, if set.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:242 PlatformInfo.Builder.setParent
    pub fn set_parent(&mut self, parent: Option<PlatformInfo>) -> &mut Self {
        match &parent {
            None => {
                self.constraints.parent(None);
                self.exec_properties_builder.set_parent(None);
            }
            Some(p) => {
                self.constraints.parent(Some(p.constraints.clone()));
                self.exec_properties_builder
                    .set_parent(Some(p.exec_properties.clone()));
            }
        }
        self.parent = parent;
        self
    }

    /// Sets the label for this [`PlatformInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:261 PlatformInfo.Builder.setLabel
    pub fn set_label(&mut self, label: Label) -> &mut Self {
        self.label = Some(label);
        self
    }

    /// Adds the given constraint value to the constraints that define this [`PlatformInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:273 PlatformInfo.Builder.addConstraint
    pub fn add_constraint(&mut self, constraint: ConstraintValueInfo) -> &mut Self {
        self.constraints.add_constraints([constraint]);
        self
    }

    /// Adds the given constraint values to the constraints that define this [`PlatformInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:285 PlatformInfo.Builder.addConstraints
    pub fn add_constraints(
        &mut self,
        constraints: impl IntoIterator<Item = ConstraintValueInfo>,
    ) -> &mut Self {
        self.constraints.add_constraints(constraints);
        self
    }

    /// Sets the data being sent to a potential remote executor. If there is a parent set, the
    /// literal string "{PARENT_REMOTE_EXECUTION_PROPERTIES}" will be replaced by the remote
    /// execution properties from that parent. Also if the parent is set, and this instance's
    /// remote execution properties are blank or unset, the parent's will be used directly.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:314 PlatformInfo.Builder.setRemoteExecutionProperties
    pub fn set_remote_execution_properties(&mut self, properties: Option<String>) -> &mut Self {
        self.remote_execution_properties = properties;
        self
    }

    /// Sets the execution properties.
    ///
    /// If there is a parent set, then all parent's properties will be inherited. Any properties
    /// included in both will use the child's value. Use the value of empty string to unset a
    /// property.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:327 PlatformInfo.Builder.setExecProperties
    pub fn set_exec_properties(&mut self, properties: IndexMap<String, String>) -> &mut Self {
        self.exec_properties_builder.set_properties(properties);
        self
    }

    /// Add the given flags to this [`PlatformInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:334 PlatformInfo.Builder.addFlags
    pub fn add_flags(&mut self, flags: impl IntoIterator<Item = String>) -> &mut Self {
        self.flags.extend(flags);
        self
    }

    /// Add the given settings to this [`PlatformInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:341 PlatformInfo.Builder.addRequiredSettings
    pub fn add_required_settings(
        &mut self,
        required_settings: impl IntoIterator<Item = ConfigMatchingProvider>,
    ) -> &mut Self {
        self.required_settings.extend(required_settings);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:347 PlatformInfo.Builder.checkToolchainTypes
    pub fn check_toolchain_types(&mut self, check_toolchain_types: bool) -> &mut Self {
        self.check_toolchain_types = check_toolchain_types;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:353 PlatformInfo.Builder.addAllowedToolchainTypes
    pub fn add_allowed_toolchain_types(
        &mut self,
        allowed_toolchain_types: impl IntoIterator<Item = Label>,
    ) -> &mut Self {
        self.allowed_toolchain_types.extend(allowed_toolchain_types);
        self
    }

    /// Sets an error message to display when a required toolchain cannot be resolved for this
    /// platform.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:363 PlatformInfo.Builder.setMissingToolchainErrorMessage
    pub fn set_missing_toolchain_error_message(&mut self, message: Option<String>) -> &mut Self {
        self.missing_toolchain_error_message = match message {
            None => None,
            Some(m) if m.is_empty() => None,
            Some(m) => Some(m),
        };
        self
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:372 PlatformInfo.Builder.checkRemoteExecutionProperties
    fn check_remote_execution_properties(
        parent: Option<&PlatformInfo>,
        remote_execution_properties: Option<&str>,
        exec_properties: &IndexMap<String, String>,
    ) -> Result<(), ExecPropertiesException> {
        let remote_set = remote_execution_properties.is_some_and(|s| !s.is_empty());
        if !exec_properties.is_empty() && remote_set {
            return Err(ExecPropertiesException::new(
                "Platform contains both remote_execution_properties and exec_properties. Prefer \
                 exec_properties over the deprecated remote_execution_properties.",
            ));
        }
        if let Some(parent) = parent {
            if !exec_properties.is_empty() && !parent.remote_execution_properties().is_empty() {
                return Err(ExecPropertiesException::new(format!(
                    "Platform specifies exec_properties but its parent {} specifies \
                     remote_execution_properties. Prefer exec_properties over the deprecated \
                     remote_execution_properties.",
                    label_to_string(parent.label.as_ref())
                )));
            }
            if remote_set && !parent.exec_properties().is_empty() {
                return Err(ExecPropertiesException::new(
                    "Platform specifies remote_execution_properties but its parent specifies \
                     exec_properties. Prefer exec_properties over the deprecated \
                     remote_execution_properties.",
                ));
            }
        }
        Ok(())
    }

    /// Returns the new [`PlatformInfo`] instance.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:407 PlatformInfo.Builder.build
    pub fn build(&self) -> Result<PlatformInfo, PlatformInfoBuildError> {
        Self::check_remote_execution_properties(
            self.parent.as_ref(),
            self.remote_execution_properties.as_deref(),
            self.exec_properties_builder.get_properties(),
        )?;

        // Merge the remote execution properties.
        let remote_execution_properties = Self::merge_remote_execution_properties(
            self.parent.as_ref(),
            self.remote_execution_properties.as_deref(),
        );

        // Merge parent flags.
        let mut flags = Vec::new();
        if let Some(parent) = &self.parent {
            flags.extend(parent.flags.iter().cloned());
        }
        flags.extend(self.flags.iter().cloned());

        // Do not merge required settings, they are not inherited.

        Ok(PlatformInfo {
            label: self.label.clone(),
            constraints: self.constraints.build()?,
            remote_execution_properties,
            exec_properties: self.exec_properties_builder.build(),
            flags,
            required_settings: self.required_settings.clone(),
            check_toolchain_types: self.check_toolchain_types,
            allowed_toolchain_types: self.allowed_toolchain_types.clone(),
            missing_toolchain_error_message: self.missing_toolchain_error_message.clone(),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:439 PlatformInfo.Builder.mergeRemoteExecutionProperties
    fn merge_remote_execution_properties(
        parent: Option<&PlatformInfo>,
        remote_execution_properties: Option<&str>,
    ) -> String {
        let mut parent_remote_execution_properties = "";
        if let Some(parent) = parent {
            parent_remote_execution_properties = parent.remote_execution_properties();
        }

        let Some(remote_execution_properties) = remote_execution_properties else {
            return parent_remote_execution_properties.to_owned();
        };

        // Java: StringUtilities.replaceAllLiteral.
        remote_execution_properties.replace(
            PlatformInfo::PARENT_REMOTE_EXECUTION_KEY,
            parent_remote_execution_properties,
        )
    }
}

/// Exception that indicates something is wrong in exec_properties configuration.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:456 PlatformInfo.ExecPropertiesException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExecPropertiesException {
    message: String,
}

impl ExecPropertiesException {
    fn new(message: impl Into<String>) -> Self {
        ExecPropertiesException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// The checked exceptions of `PlatformInfo.Builder.build`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlatformInfoBuildError {
    #[error(transparent)]
    DuplicateConstraint(#[from] DuplicateConstraintException),
    #[error(transparent)]
    ExecProperties(#[from] ExecPropertiesException),
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/PlatformInfoApi.java:37 PlatformInfoApi
const FIELDS: [&str; 2] = ["constraints", "label"];
/// The fields with `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API` (PlatformInfoApi.java:42,
/// :51): absent when the flag is off.
const GATED: [&str; 2] = ["constraints", "label"];

#[starlark_value(type = "PlatformInfo")]
impl<'v> StarlarkValue<'v> for PlatformInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "label" => Some(match &self.label {
                Some(l) => heap.alloc(LabelValue(l.clone())),
                None => Value::new_none(),
            }),
            "constraints" => Some(heap.alloc(self.constraints.clone())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        FIELDS.contains(&attribute)
    }

    fn get_attr_fallible(
        &self,
        attribute: Hashed<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        gated_get_attr(self, &GATED, attribute.key(), eval)
    }

    fn has_attr_fallible(
        &self,
        attribute: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        gated_has_attr(self, &GATED, attribute, eval)
    }

    // PORT: Java's `dir` leaves out the GATED fields when the flag is off
    // (CallUtils.getAnnotatedMethods); `dir_attr` has no evaluator, so they are listed.

    fn dir_attr(&self) -> Vec<String> {
        FIELDS.iter().map(|s| (*s).to_owned()).collect()
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<PlatformInfo>()
            .is_some_and(|o| o == self))
    }

    // Java: NativeInfo.isImmutable is true (hashable).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        label_to_string(self.label.as_ref()).hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(PLATFORM_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::constraint_setting_info::ConstraintSettingInfo;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    fn map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn cv(setting: &ConstraintSettingInfo, l: &str) -> ConstraintValueInfo {
        ConstraintValueInfo::create(setting.clone(), label(l))
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java builder
    #[test]
    fn builder() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:s1"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:s2"));
        let mut builder = PlatformInfo::builder();
        builder.add_constraint(cv(&setting1, "//constraint:v1"));
        builder.add_constraint(cv(&setting2, "//constraint:v2"));
        let platform_info = builder.build().unwrap();
        assert!(platform_info.constraints().has(&setting1));
        assert_eq!(
            platform_info.constraints().get(&setting1).unwrap().label(),
            &label("//constraint:v1")
        );
        assert!(platform_info.constraints().has(&setting2));
        assert_eq!(
            platform_info.constraints().get(&setting2).unwrap().label(),
            &label("//constraint:v2")
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java constraints_parentPlatform_noOverlaps
    #[test]
    fn constraints_parent_platform_no_overlaps() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:s1"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:s2"));
        let setting3 = ConstraintSettingInfo::create(label("//constraint:s3"));
        let parent = PlatformInfo::builder()
            .add_constraint(cv(&setting1, "//constraint:v1"))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.add_constraint(cv(&setting2, "//constraint:v2"));
        builder.add_constraint(cv(&setting3, "//constraint:v3"));
        let platform_info = builder.build().unwrap();
        for (s, v) in [
            (&setting1, "//constraint:v1"),
            (&setting2, "//constraint:v2"),
            (&setting3, "//constraint:v3"),
        ] {
            assert!(platform_info.constraints().has(s));
            assert_eq!(
                platform_info.constraints().get(s).unwrap().label(),
                &label(v)
            );
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java constraints_parentPlatform_overlaps
    #[test]
    fn constraints_parent_platform_overlaps() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:s1"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:s2"));
        let setting3 = ConstraintSettingInfo::create(label("//constraint:s3"));
        let parent = PlatformInfo::builder()
            .add_constraint(cv(&setting1, "//constraint:v1"))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.add_constraint(cv(&setting1, "//constraint:v1a"));
        builder.add_constraint(cv(&setting2, "//constraint:v2"));
        builder.add_constraint(cv(&setting3, "//constraint:v3"));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.constraints().get(&setting1).unwrap().label(),
            &label("//constraint:v1a")
        );
        assert_eq!(
            platform_info.constraints().get(&setting2).unwrap().label(),
            &label("//constraint:v2")
        );
        assert_eq!(
            platform_info.constraints().get(&setting3).unwrap().label(),
            &label("//constraint:v3")
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java constraints_overlappingError
    #[test]
    fn constraints_overlapping_error() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:basic"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:complex"));
        let setting3 = ConstraintSettingInfo::create(label("//constraint:single"));
        let mut builder = PlatformInfo::builder();
        builder.add_constraint(cv(&setting1, "//constraint:value1"));
        builder.add_constraint(cv(&setting1, "//constraint:value2"));
        builder.add_constraint(cv(&setting2, "//constraint:value3"));
        builder.add_constraint(cv(&setting2, "//constraint:value4"));
        builder.add_constraint(cv(&setting2, "//constraint:value5"));
        builder.add_constraint(cv(&setting3, "//constraint:value6"));
        let e = builder.build().unwrap_err();
        assert!(matches!(e, PlatformInfoBuildError::DuplicateConstraint(_)));
        assert!(e.to_string().contains(
            "Duplicate constraint values detected: \
             constraint_setting //constraint:basic has \
             [//constraint:value1, //constraint:value2], \
             constraint_setting //constraint:complex has \
             [//constraint:value3, //constraint:value4, //constraint:value5]"
        ));
    }

    fn with_remote(props: &str) -> PlatformInfo {
        PlatformInfo::builder()
            .set_remote_execution_properties(Some(props.to_owned()))
            .build()
            .unwrap()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties
    #[test]
    fn remote_execution_properties() {
        assert_eq!(
            with_remote("properties").remote_execution_properties(),
            "properties"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentPlatform_keep
    #[test]
    fn remote_execution_properties_parent_platform_keep() {
        let parent = with_remote("parent properties");
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.remote_execution_properties(),
            "parent properties"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentPlatform_override
    #[test]
    fn remote_execution_properties_parent_platform_override() {
        let parent = with_remote("parent properties");
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.set_remote_execution_properties(Some("child properties".to_owned()));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.remote_execution_properties(),
            "child properties"
        );
        assert!(platform_info.exec_properties().is_empty());
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentPlatform_merge
    #[test]
    fn remote_execution_properties_parent_platform_merge() {
        let parent = with_remote("parent properties");
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.set_remote_execution_properties(Some(
            "child {PARENT_REMOTE_EXECUTION_PROPERTIES} properties".to_owned(),
        ));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.remote_execution_properties(),
            "child parent properties properties"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentPlatform_merge_noParent
    #[test]
    fn remote_execution_properties_parent_platform_merge_no_parent() {
        let platform_info = with_remote("child {PARENT_REMOTE_EXECUTION_PROPERTIES} properties");
        assert_eq!(
            platform_info.remote_execution_properties(),
            "child  properties"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentPlatform_merge_parentNotSet
    #[test]
    fn remote_execution_properties_parent_platform_merge_parent_not_set() {
        let parent = PlatformInfo::builder().build().unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.set_remote_execution_properties(Some(
            "child {PARENT_REMOTE_EXECUTION_PROPERTIES} properties".to_owned(),
        ));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.remote_execution_properties(),
            "child  properties"
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java remoteExecutionProperties_parentSpecifiesExecProperties_error
    #[test]
    fn remote_execution_properties_parent_specifies_exec_properties_error() {
        let parent = PlatformInfo::builder()
            .set_label(label("//foo:parent_platform"))
            .set_exec_properties(map(&[("elem1", "value1")]))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.set_remote_execution_properties(Some("props".to_owned()));
        let e = builder.build().unwrap_err();
        assert!(matches!(e, PlatformInfoBuildError::ExecProperties(_)));
        assert!(e.to_string().contains(
            "Platform specifies remote_execution_properties but its parent specifies \
             exec_properties. Prefer exec_properties over the deprecated \
             remote_execution_properties."
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_empty
    #[test]
    fn exec_properties_empty() {
        let platform_info = PlatformInfo::builder()
            .set_exec_properties(map(&[]))
            .build()
            .unwrap();
        assert!(platform_info.exec_properties().is_empty());
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_one
    #[test]
    fn exec_properties_one() {
        let platform_info = PlatformInfo::builder()
            .set_exec_properties(map(&[("elem1", "value1")]))
            .build()
            .unwrap();
        assert_eq!(
            platform_info.exec_properties(),
            &map(&[("elem1", "value1")])
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_parentPlatform_keep
    #[test]
    fn exec_properties_parent_platform_keep() {
        let parent = PlatformInfo::builder()
            .set_exec_properties(map(&[("parent", "properties")]))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.exec_properties(),
            &map(&[("parent", "properties")])
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_parentPlatform_inheritance
    #[test]
    fn exec_properties_parent_platform_inheritance() {
        let parent = PlatformInfo::builder()
            .set_exec_properties(map(&[
                ("p1", "keep"),
                ("p2", "delete"),
                ("p3", "parent"),
                ("p4", "del2"),
            ]))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        let platform_info = builder
            .set_exec_properties(map(&[("p2", ""), ("p3", "child"), ("p4", "")]))
            .build()
            .unwrap();
        assert_eq!(
            platform_info.exec_properties(),
            &map(&[("p1", "keep"), ("p3", "child")])
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_remoteExecProperties_error
    #[test]
    fn exec_properties_remote_exec_properties_error() {
        let mut builder = PlatformInfo::builder();
        builder.set_exec_properties(map(&[("elem1", "value1")]));
        builder.set_remote_execution_properties(Some("props".to_owned()));
        let e = builder.build().unwrap_err();
        assert!(e.to_string().contains(
            "Platform contains both remote_execution_properties and exec_properties. Prefer \
             exec_properties over the deprecated remote_execution_properties."
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java execProperties_parentSpecifiesRemoteExecutionProperties_error
    #[test]
    fn exec_properties_parent_specifies_remote_execution_properties_error() {
        let parent = PlatformInfo::builder()
            .set_label(label("//foo:parent_platform"))
            .set_remote_execution_properties(Some("props".to_owned()))
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.set_exec_properties(map(&[("elem1", "value1")]));
        let e = builder.build().unwrap_err();
        assert!(e.to_string().contains(
            "Platform specifies exec_properties but its parent //foo:parent_platform specifies \
             remote_execution_properties. Prefer exec_properties over the deprecated \
             remote_execution_properties."
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java flags_empty
    #[test]
    fn flags_empty() {
        assert!(PlatformInfo::builder().build().unwrap().flags().is_empty());
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java flags
    #[test]
    fn flags() {
        let platform_info = PlatformInfo::builder()
            .add_flags(["--cpu=k8".to_owned(), "--//starlark:flag=other".to_owned()])
            .build()
            .unwrap();
        assert_eq!(
            platform_info.flags(),
            ["--cpu=k8", "--//starlark:flag=other"]
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java flags_parentPlatform_keep
    #[test]
    fn flags_parent_platform_keep() {
        let parent = PlatformInfo::builder()
            .add_flags(["--cpu=k8".to_owned()])
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.add_flags(["--//starlark:flag=other".to_owned()]);
        let platform_info = builder.build().unwrap();
        assert_eq!(
            platform_info.flags(),
            ["--cpu=k8", "--//starlark:flag=other"]
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java flags_parentPlatform_inheritance
    #[test]
    fn flags_parent_platform_inheritance() {
        let parent = PlatformInfo::builder()
            .add_flags(["--cpu=arm".to_owned()])
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        builder.add_flags(["--cpu=k8".to_owned()]);
        let platform_info = builder.build().unwrap();
        assert_eq!(platform_info.flags(), ["--cpu=arm", "--cpu=k8"]);
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java requiredSettings_empty
    #[test]
    fn required_settings_empty() {
        assert!(
            PlatformInfo::builder()
                .build()
                .unwrap()
                .required_settings()
                .is_empty()
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java configMatchingProvider
    fn config_matching_provider(l: &str, matches: bool) -> ConfigMatchingProvider {
        use crate::config::config_matching_provider::{Diff, MatchResult};
        ConfigMatchingProvider::create(
            label(l),
            Vec::new(),
            IndexMap::new(),
            indexmap::IndexSet::new(),
            if matches {
                MatchResult::Match
            } else {
                MatchResult::NoMatch(vec![Diff::new(label("//fake"), "bar", "foo")])
            },
        )
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java requiredSettings
    #[test]
    fn required_settings() {
        let platform_info = PlatformInfo::builder()
            .add_required_settings([
                config_matching_provider("//setting:first", true),
                config_matching_provider("//setting:second", false),
            ])
            .build()
            .unwrap();
        assert_eq!(platform_info.required_settings().len(), 2);
        assert_eq!(
            platform_info
                .required_settings()
                .iter()
                .map(|s| s.label().clone())
                .collect::<Vec<_>>(),
            vec![label("//setting:first"), label("//setting:second")]
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java requiredSettings_parentPlatform_areIgnored
    #[test]
    fn required_settings_parent_platform_are_ignored() {
        let parent = PlatformInfo::builder()
            .add_required_settings([
                config_matching_provider("//setting:first", true),
                config_matching_provider("//setting:second", false),
            ])
            .build()
            .unwrap();
        let mut builder = PlatformInfo::builder();
        builder.set_parent(Some(parent));
        let platform_info = builder.build().unwrap();
        assert!(platform_info.required_settings().is_empty());
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfoTest.java equalsTester
    #[test]
    fn equals_tester() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:basic"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:other"));
        let value1 = cv(&setting1, "//constraint:value1");
        let value2 = cv(&setting2, "//constraint:value2");
        let value3 = cv(&setting2, "//constraint:value3");
        let make = |l: &str, values: &[&ConstraintValueInfo]| {
            PlatformInfo::builder()
                .set_label(label(l))
                .add_constraints(values.iter().map(|v| (*v).clone()))
                .build()
                .unwrap()
        };
        // Base case.
        let a = make("//platform/plat1", &[&value1, &value2]);
        let b = make("//platform/plat1", &[&value1, &value2]);
        let groups = [
            // Different label.
            make("//platform/plat2", &[&value1, &value2]),
            // Extra constraint.
            make("//platform/plat1", &[&value1, &value3]),
            // Missing constraint.
            make("//platform/plat1", &[&value1]),
            // Different remote exec properties.
            PlatformInfo::builder()
                .set_label(label("//platform/plat1"))
                .add_constraints([value1.clone(), value2.clone()])
                .set_remote_execution_properties(Some("foo".to_owned()))
                .build()
                .unwrap(),
            // Different no toolchain error message.
            PlatformInfo::builder()
                .set_label(label("//platform/plat1"))
                .add_constraints([value1.clone(), value2.clone()])
                .set_missing_toolchain_error_message(Some(
                    "Check docs for plat1 at http://example.com/plat1".to_owned(),
                ))
                .build()
                .unwrap(),
        ];
        assert_eq!(a, b);
        for (i, g) in groups.iter().enumerate() {
            assert_ne!(&a, g);
            for (j, h) in groups.iter().enumerate() {
                assert_eq!(i == j, g == h);
            }
        }
    }

    #[test]
    fn empty_platform_info() {
        assert_eq!(
            EMPTY_PLATFORM_INFO.label(),
            &label("@bazel_tools//tools:internal_platform")
        );
    }
}
