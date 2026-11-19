//! Port of `src/test/java/com/google/devtools/build/lib/rules/platform/PlatformTestCase.java`:
//! builders that write platforms and constraints, on a [`BuildViewTestCase`].

use blaze_analysis::platform::constraint_setting_info::ConstraintSettingInfo;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_cmdline::Label;
use indexmap::IndexMap;

use crate::testing::BuildViewTestCase;
use crate::testing::build_view_test_case::parse_label;

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:35 PlatformTestCase.constraintBuilder
pub fn constraint_builder(name: &str) -> ConstraintBuilder {
    ConstraintBuilder::new(name)
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:39 PlatformTestCase.platformBuilder
pub fn platform_builder(name: &str) -> PlatformBuilder {
    PlatformBuilder::new(name)
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:44 PlatformTestCase.fetchConstraintSettingInfo
pub fn fetch_constraint_setting_info(
    t: &mut BuildViewTestCase,
    label: &str,
) -> Option<ConstraintSettingInfo> {
    let target = t.get_configured_target(label);
    platform_provider_utils::constraint_setting(target.as_ref())
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:50 PlatformTestCase.fetchPlatformInfo
pub fn fetch_platform_info(
    t: &mut BuildViewTestCase,
    platform_label: &str,
) -> Option<PlatformInfo> {
    let target = t.get_configured_target(platform_label);
    platform_provider_utils::platform(target.as_ref())
}

/// `scratch.appendFile(label's package + "/BUILD", lines)`.
fn append_to_build_file(t: &BuildViewTestCase, label: &Label, lines: &[String]) {
    let filename = label
        .get_package_fragment()
        .get_relative_str("BUILD")
        .get_path_string()
        .to_string();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    t.scratch.append_file(&filename, &lines);
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:55 PlatformTestCase.ConstraintBuilder
pub struct ConstraintBuilder {
    label: Label,
    constraint_values: Vec<String>,
    default_constraint_value: Option<String>,
}

impl ConstraintBuilder {
    pub fn new(name: &str) -> ConstraintBuilder {
        ConstraintBuilder {
            label: parse_label(name),
            constraint_values: Vec::new(),
            default_constraint_value: None,
        }
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:65 PlatformTestCase.ConstraintBuilder.defaultConstraintValue
    pub fn default_constraint_value(mut self, default_constraint_value: &str) -> Self {
        self.default_constraint_value = Some(default_constraint_value.to_string());
        self.constraint_values
            .push(default_constraint_value.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:72 PlatformTestCase.ConstraintBuilder.addConstraintValue
    pub fn add_constraint_value(mut self, constraint_value: &str) -> Self {
        self.constraint_values.push(constraint_value.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:77 PlatformTestCase.ConstraintBuilder.lines
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        // Add the constraint setting.
        lines.push(format!(
            "constraint_setting(name = '{}',",
            self.label.get_name()
        ));
        if let Some(default) = self
            .default_constraint_value
            .as_ref()
            .filter(|d| !d.is_empty())
        {
            lines.push(format!("  default_constraint_value = ':{default}',"));
        }
        lines.push(")".to_string());

        // Add the constraint values.
        for constraint_value in &self.constraint_values {
            lines.push("constraint_value(".to_string());
            lines.push(format!("  name = '{constraint_value}',"));
            lines.push(format!(
                "  constraint_setting = ':{}',",
                self.label.get_name()
            ));
            lines.push(")".to_string());
        }

        lines
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:99 PlatformTestCase.ConstraintBuilder.write
    pub fn write(&self, t: &BuildViewTestCase) {
        append_to_build_file(t, &self.label, &self.lines());
    }
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:106 PlatformTestCase.PlatformBuilder
pub struct PlatformBuilder {
    label: Label,
    constraint_values: Vec<String>,
    parent_label: Option<Label>,
    remote_execution_properties: String,
    exec_properties: Option<IndexMap<String, String>>,
    flags: Vec<String>,
}

impl PlatformBuilder {
    pub fn new(name: &str) -> PlatformBuilder {
        PlatformBuilder {
            label: parse_label(name),
            constraint_values: Vec::new(),
            parent_label: None,
            remote_execution_properties: String::new(),
            exec_properties: None,
            flags: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:119 PlatformTestCase.PlatformBuilder.setParent
    pub fn set_parent(mut self, parent_label: &str) -> Self {
        self.parent_label = Some(parse_label(parent_label));
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:125 PlatformTestCase.PlatformBuilder.addConstraint
    pub fn add_constraint(mut self, value: &str) -> Self {
        self.constraint_values.push(value.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:131 PlatformTestCase.PlatformBuilder.setRemoteExecutionProperties
    pub fn set_remote_execution_properties(mut self, value: &str) -> Self {
        self.remote_execution_properties = value.to_string();
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:137 PlatformTestCase.PlatformBuilder.setExecProperties
    pub fn set_exec_properties(mut self, value: IndexMap<String, String>) -> Self {
        self.exec_properties = Some(value);
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:143 PlatformTestCase.PlatformBuilder.addFlags
    pub fn add_flags(mut self, flags: &[&str]) -> Self {
        self.flags.extend(flags.iter().map(|f| f.to_string()));
        self
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:148 PlatformTestCase.PlatformBuilder.lines
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        lines.push("platform(".to_string());
        lines.push(format!("  name = '{}',", self.label.get_name()));
        if let Some(parent_label) = &self.parent_label {
            lines.push(format!("  parents = ['{parent_label}'],"));
        }
        lines.push("  constraint_values = [".to_string());
        for name in &self.constraint_values {
            lines.push(format!("    ':{name}',"));
        }
        lines.push("  ],".to_string());
        if !self.remote_execution_properties.is_empty() {
            lines.push(format!(
                "  remote_execution_properties = '{}',",
                self.remote_execution_properties
            ));
        }
        if let Some(exec_properties) = self.exec_properties.as_ref().filter(|p| !p.is_empty()) {
            lines.push("  exec_properties = { ".to_string());
            for (key, value) in exec_properties {
                lines.push(format!("    \"{key}\": \"{value}\","));
            }
            lines.push("  }".to_string());
        }
        if !self.flags.is_empty() {
            lines.push("  flags = [".to_string());
            for flag in &self.flags {
                lines.push(format!("    '{flag}',"));
            }
            lines.push("  ],".to_string());
        }
        lines.push(")".to_string());

        lines
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformTestCase.java:182 PlatformTestCase.PlatformBuilder.write
    pub fn write(&self, t: &BuildViewTestCase) {
        append_to_build_file(t, &self.label, &self.lines());
    }
}
