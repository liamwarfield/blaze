//! Port of `analysis/platform/DeclaredToolchainInfo.java`.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use blaze_cmdline::Label;

use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::platform::constraint_collection::{ConstraintCollection, DuplicateConstraintException};
use crate::platform::constraint_value_info::ConstraintValueInfo;
use crate::platform::toolchain_type_info::ToolchainTypeInfo;
use crate::provider_map::TransitiveInfoProvider;

/// Provider for a toolchain declaration, which associates a toolchain type, the execution and
/// target constraints, and the actual toolchain label. The toolchain is then available for use
/// but will be lazily resolved only when it is actually needed for toolchain-aware rules.
/// Toolchain definitions are exposed to Starlark and Bazel via `ToolchainInfo` providers.
// Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:43 DeclaredToolchainInfo
#[derive(Clone, Debug)]
pub struct DeclaredToolchainInfo {
    /// The type of the toolchain being declared. This will be a label of a toolchain_type()
    /// target.
    toolchain_type: ToolchainTypeInfo,
    /// The constraints describing the execution environment.
    exec_constraints: Arc<ConstraintCollection>,
    /// The constraints describing the target environment.
    target_constraints: Arc<ConstraintCollection>,
    /// The setting, that target build configuration needs to satisfy.
    target_settings: Vec<ConfigMatchingProvider>,
    /// The label of the `toolchain` target itself.
    target_label: Label,
    /// The label of the toolchain to resolve for use in toolchain-aware rules.
    resolved_toolchain_label: Label,
}

// Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:67 DeclaredToolchainInfo.USE_TARGET_PLATFORM_CONSTRAINTS
static USE_TARGET_PLATFORM_CONSTRAINTS: LazyLock<Arc<ConstraintCollection>> = LazyLock::new(|| {
    Arc::new(
        ConstraintCollection::builder()
            .build()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}")),
    )
});

impl DeclaredToolchainInfo {
    pub fn toolchain_type(&self) -> &ToolchainTypeInfo {
        &self.toolchain_type
    }

    pub fn exec_constraints(&self) -> &ConstraintCollection {
        &self.exec_constraints
    }

    pub fn target_constraints(&self) -> &ConstraintCollection {
        &self.target_constraints
    }

    pub fn target_settings(&self) -> &[ConfigMatchingProvider] {
        &self.target_settings
    }

    pub fn target_label(&self) -> &Label {
        &self.target_label
    }

    pub fn resolved_toolchain_label(&self) -> &Label {
        &self.resolved_toolchain_label
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:60 DeclaredToolchainInfo.hasTargetToExecConstraints
    pub fn has_target_to_exec_constraints(&self) -> bool {
        // Since use_target_platform_constraints is mutually exclusive with exec_compatible_with
        // and target_compatible_with, checking one of the two would be enough.
        Arc::ptr_eq(&self.exec_constraints, &USE_TARGET_PLATFORM_CONSTRAINTS)
            && Arc::ptr_eq(&self.target_constraints, &USE_TARGET_PLATFORM_CONSTRAINTS)
    }

    /// Returns a new builder for creating [`DeclaredToolchainInfo`] instances.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:184 DeclaredToolchainInfo.builder
    pub fn builder() -> DeclaredToolchainInfoBuilder {
        DeclaredToolchainInfoBuilder::default()
    }
}

// Java: record equals of DeclaredToolchainInfo.
impl PartialEq for DeclaredToolchainInfo {
    fn eq(&self, other: &Self) -> bool {
        self.toolchain_type == other.toolchain_type
            && *self.exec_constraints == *other.exec_constraints
            && *self.target_constraints == *other.target_constraints
            && self.target_settings == other.target_settings
            && self.target_label == other.target_label
            && self.resolved_toolchain_label == other.resolved_toolchain_label
    }
}

impl Eq for DeclaredToolchainInfo {}

impl TransitiveInfoProvider for DeclaredToolchainInfo {
    fn java_class_name(&self) -> &'static str {
        "DeclaredToolchainInfo"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Builder class to assist in creating [`DeclaredToolchainInfo`] instances.
// Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:78 DeclaredToolchainInfo.Builder
#[derive(Clone, Debug, Default)]
pub struct DeclaredToolchainInfoBuilder {
    toolchain_type: Option<ToolchainTypeInfo>,
    exec_constraints: Vec<ConstraintValueInfo>,
    target_constraints: Vec<ConstraintValueInfo>,
    target_settings: Vec<ConfigMatchingProvider>,
    target_label: Option<Label>,
    resolved_toolchain_label: Option<Label>,
}

impl DeclaredToolchainInfoBuilder {
    /// Sets the type of the toolchain being declared.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:89 DeclaredToolchainInfo.Builder.toolchainType
    pub fn toolchain_type(&mut self, toolchain_type: Option<ToolchainTypeInfo>) -> &mut Self {
        self.toolchain_type = toolchain_type;
        self
    }

    /// Adds constraints describing the execution environment.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:96 DeclaredToolchainInfo.Builder.addExecConstraints
    pub fn add_exec_constraints(
        &mut self,
        constraints: impl IntoIterator<Item = ConstraintValueInfo>,
    ) -> &mut Self {
        self.exec_constraints.extend(constraints);
        self
    }

    /// Adds constraints describing the target environment.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:109 DeclaredToolchainInfo.Builder.addTargetConstraints
    pub fn add_target_constraints(
        &mut self,
        constraints: impl IntoIterator<Item = ConstraintValueInfo>,
    ) -> &mut Self {
        self.target_constraints.extend(constraints);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:121 DeclaredToolchainInfo.Builder.addTargetSettings
    pub fn add_target_settings(
        &mut self,
        target_settings: impl IntoIterator<Item = ConfigMatchingProvider>,
    ) -> &mut Self {
        self.target_settings.extend(target_settings);
        self
    }

    /// Sets the label of the `toolchain` target itself.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:128 DeclaredToolchainInfo.Builder.targetLabel
    pub fn target_label(&mut self, target_label: Label) -> &mut Self {
        self.target_label = Some(target_label);
        self
    }

    /// Sets the label of the toolchain to resolve for use in toolchain-aware rules.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:135 DeclaredToolchainInfo.Builder.resolvedToolchainLabel
    pub fn resolved_toolchain_label(&mut self, resolved_toolchain_label: Label) -> &mut Self {
        self.resolved_toolchain_label = Some(resolved_toolchain_label);
        self
    }

    /// Java's record constructor (`requireNonNull` on every component).
    fn create(
        &self,
        exec_constraints: Arc<ConstraintCollection>,
        target_constraints: Arc<ConstraintCollection>,
    ) -> DeclaredToolchainInfo {
        DeclaredToolchainInfo {
            toolchain_type: self
                .toolchain_type
                .clone()
                .expect("NullPointerException: toolchainType"),
            exec_constraints,
            target_constraints,
            target_settings: self.target_settings.clone(),
            target_label: self
                .target_label
                .clone()
                .expect("NullPointerException: targetLabel"),
            resolved_toolchain_label: self
                .resolved_toolchain_label
                .clone()
                .expect("NullPointerException: resolvedToolchainLabel"),
        }
    }

    /// Returns the newly created [`DeclaredToolchainInfo`] instance.
    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:141 DeclaredToolchainInfo.Builder.build
    pub fn build(
        &self,
    ) -> Result<DeclaredToolchainInfo, DeclaredToolchainInfoDuplicateConstraintException> {
        // Handle constraint duplication in attributes separately, so they can be reported
        // correctly.
        let mut exec_builder = ConstraintCollection::builder();
        exec_builder.add_constraints(self.exec_constraints.iter().cloned());
        let (exec_constraints, exec_constraints_exception) = match exec_builder.build() {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e)),
        };
        let mut target_builder = ConstraintCollection::builder();
        target_builder.add_constraints(self.target_constraints.iter().cloned());
        let (target_constraints, target_constraints_exception) = match target_builder.build() {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e)),
        };
        if exec_constraints_exception.is_some() || target_constraints_exception.is_some() {
            return Err(DeclaredToolchainInfoDuplicateConstraintException::new(
                exec_constraints_exception,
                target_constraints_exception,
            ));
        }
        Ok(self.create(
            Arc::new(exec_constraints.unwrap()),
            Arc::new(target_constraints.unwrap()),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:172 DeclaredToolchainInfo.Builder.buildWithTargetToExecConstraints
    pub fn build_with_target_to_exec_constraints(&self) -> DeclaredToolchainInfo {
        self.create(
            USE_TARGET_PLATFORM_CONSTRAINTS.clone(),
            USE_TARGET_PLATFORM_CONSTRAINTS.clone(),
        )
    }
}

/// Exception for reporting duplicated constraints from declared toolchains.
///
/// Contains distinct fields for errors from the execution constraints or target constraints, so
/// that these can be reported separately.
// Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:194 DeclaredToolchainInfo.DuplicateConstraintException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DeclaredToolchainInfoDuplicateConstraintException {
    exec_constraints_exception: Option<DuplicateConstraintException>,
    target_constraints_exception: Option<DuplicateConstraintException>,
    message: String,
}

impl DeclaredToolchainInfoDuplicateConstraintException {
    fn new(
        exec_constraints_exception: Option<DuplicateConstraintException>,
        target_constraints_exception: Option<DuplicateConstraintException>,
    ) -> Self {
        let message = Self::format_error(
            exec_constraints_exception.as_ref(),
            target_constraints_exception.as_ref(),
        );
        DeclaredToolchainInfoDuplicateConstraintException {
            exec_constraints_exception,
            target_constraints_exception,
            message,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:210 DeclaredToolchainInfo.DuplicateConstraintException.execConstraintsException
    pub fn exec_constraints_exception(&self) -> Option<&DuplicateConstraintException> {
        self.exec_constraints_exception.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:214 DeclaredToolchainInfo.DuplicateConstraintException.targetConstraintsException
    pub fn target_constraints_exception(&self) -> Option<&DuplicateConstraintException> {
        self.target_constraints_exception.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfo.java:218 DeclaredToolchainInfo.DuplicateConstraintException.formatError
    pub fn format_error(
        exec_constraints_exception: Option<&DuplicateConstraintException>,
        target_constraints_exception: Option<&DuplicateConstraintException>,
    ) -> String {
        let mut message = String::new();
        message.push_str("Duplicate constraints detected[");
        if let Some(e) = exec_constraints_exception {
            message.push_str("in execution constraints: ");
            message.push_str(e.get_message());
        }
        if let Some(e) = target_constraints_exception {
            if exec_constraints_exception.is_some() {
                message.push_str(", ");
            }
            message.push_str("in target constraints: ");
            message.push_str(e.get_message());
        }
        message.push(']');
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::constraint_setting_info::ConstraintSettingInfo;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfoTest.java toolchainInfo_overlappingConstraintsError
    #[test]
    fn toolchain_info_overlapping_constraints_error() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:basic"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:complex"));
        let mut builder = DeclaredToolchainInfo::builder();
        builder.add_exec_constraints([ConstraintValueInfo::create(
            setting1.clone(),
            label("//constraint:value1"),
        )]);
        builder.add_exec_constraints([ConstraintValueInfo::create(
            setting1.clone(),
            label("//constraint:value2"),
        )]);
        for v in [
            "//constraint:value3",
            "//constraint:value4",
            "//constraint:value5",
        ] {
            builder
                .add_target_constraints([ConstraintValueInfo::create(setting2.clone(), label(v))]);
        }
        let exception = builder.build().unwrap_err();
        assert!(
            exception
                .exec_constraints_exception()
                .unwrap()
                .get_message()
                .contains(
                    "Duplicate constraint values detected: \
                     constraint_setting //constraint:basic has \
                     [//constraint:value1, //constraint:value2]"
                )
        );
        assert!(
            exception
                .target_constraints_exception()
                .unwrap()
                .get_message()
                .contains(
                    "Duplicate constraint values detected: \
                     constraint_setting //constraint:complex has \
                     [//constraint:value3, //constraint:value4, //constraint:value5]"
                )
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/DeclaredToolchainInfoTest.java toolchainInfo_equalsTester
    #[test]
    fn toolchain_info_equals_tester() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:setting1"));
        let constraint1 = ConstraintValueInfo::create(setting1.clone(), label("//constraint:foo"));
        let constraint2 = ConstraintValueInfo::create(setting1, label("//constraint:bar"));
        let make = |tc: &str,
                    exec: &ConstraintValueInfo,
                    target: &ConstraintValueInfo,
                    def: &str,
                    tl: &str| {
            DeclaredToolchainInfo::builder()
                .toolchain_type(Some(ToolchainTypeInfo::create(label(tc))))
                .add_exec_constraints([exec.clone()])
                .add_target_constraints([target.clone()])
                .resolved_toolchain_label(label(def))
                .target_label(label(tl))
                .build()
                .unwrap()
        };
        // Base case.
        let a = make(
            "//toolchain:tc1",
            &constraint1,
            &constraint2,
            "//toolchain:toolchain_def1",
            "//toolchain:toolchain1",
        );
        let b = make(
            "//toolchain:tc1",
            &constraint1,
            &constraint2,
            "//toolchain:toolchain_def1",
            "//toolchain:toolchain1",
        );
        let groups = [
            // Different type.
            make(
                "//toolchain:tc2",
                &constraint1,
                &constraint2,
                "//toolchain:toolchain_def1",
                "//toolchain:toolchain1",
            ),
            // Different constraints.
            make(
                "//toolchain:tc1",
                &constraint2,
                &constraint1,
                "//toolchain:toolchain_def1",
                "//toolchain:toolchain1",
            ),
            // Different toolchain label.
            make(
                "//toolchain:tc1",
                &constraint1,
                &constraint2,
                "//toolchain:toolchain_def2",
                "//toolchain:toolchain1",
            ),
            // Different target label.
            make(
                "//toolchain:tc1",
                &constraint1,
                &constraint2,
                "//toolchain:toolchain_def1",
                "//toolchain:toolchain2",
            ),
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
    fn has_target_to_exec_constraints() {
        let mut builder = DeclaredToolchainInfo::builder();
        builder
            .toolchain_type(Some(ToolchainTypeInfo::create(label("//toolchain:tc1"))))
            .resolved_toolchain_label(label("//toolchain:def"))
            .target_label(label("//toolchain:t"));
        assert!(
            builder
                .build_with_target_to_exec_constraints()
                .has_target_to_exec_constraints()
        );
        assert!(!builder.build().unwrap().has_target_to_exec_constraints());
    }
}
