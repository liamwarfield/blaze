//! Port of `lib/actions/ActionOwner.java`, with the parts of `BuildConfigurationInfo`,
//! `BuildConfigurationEvent` and `AspectDescriptor` it carries.

use std::fmt;

use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_protos::build_event_stream;
use indexmap::IndexMap;

use super::platform_info::PlatformInfoRef;

/// The `BuildConfigurationEvent` of a configuration: its event id (the configuration checksum) and
/// the BEP `Configuration` payload.
// Java: com/google/devtools/build/lib/actions/BuildConfigurationEvent.java:29 BuildConfigurationEvent
// PORT: Java stores the full `BuildEventId` and `BuildEvent` protos; the configuration event id is
// the checksum, and the payload is the `configuration` field.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildConfigurationEvent {
    /// `getEventId().getConfiguration().getId()`.
    pub event_id: String,
    /// `asStreamProto(...).getConfiguration()`.
    pub configuration: build_event_stream::Configuration,
}

/// The parts of a configuration that actions see.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationInfo.java:31 BuildConfigurationInfo
// PORT: Java's interface is implemented by `BuildConfigurationValue` (blaze_build_config) and by
// `AutoBuildConfigurationInfo`; a plain struct here, built by the analysis layer from its
// configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildConfigurationInfo {
    pub mnemonic: String,
    pub checksum: String,
    pub build_configuration_event: Option<BuildConfigurationEvent>,
    pub is_tool_configuration: bool,
    /// `getCommandLineLimits().maxLength`.
    pub command_line_limits_max_length: i32,
}

impl BuildConfigurationInfo {
    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationInfo.java:61 AutoBuildConfigurationInfo.create
    pub fn create(
        mnemonic: impl Into<String>,
        checksum: impl Into<String>,
        build_configuration_event: Option<BuildConfigurationEvent>,
        is_tool_configuration: bool,
    ) -> BuildConfigurationInfo {
        BuildConfigurationInfo {
            mnemonic: mnemonic.into(),
            checksum: checksum.into(),
            build_configuration_event,
            is_tool_configuration,
            // Java: AutoBuildConfigurationInfo.getCommandLineLimits returns UNLIMITED.
            command_line_limits_max_length: i32::MAX,
        }
    }
}

/// The parts of `packages/AspectDescriptor` actions use: the aspect class name and the
/// parameters (attribute name to value, in multimap entry order).
// Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:37 AspectDescriptor
// PORT: AspectDescriptor lives in blaze_packages; the analysis layer converts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AspectDescriptor {
    /// `getAspectClass().getName()`.
    pub aspect_class_name: String,
    /// `getParameters().getAttributes().entries()`.
    pub parameters: Vec<(String, String)>,
    /// `getDescription()`.
    pub description: String,
}

/// Contains metadata used for reporting the progress and status of an action.
///
/// Morally an action's owner is the RuleConfiguredTarget instance responsible for creating it,
/// but to avoid storing heavyweight analysis objects in actions, and to avoid coupling between
/// the analysis and actions packages, the RuleConfiguredTarget provides an instance of this class.
// Java: com/google/devtools/build/lib/actions/ActionOwner.java:40 ActionOwner
// PORT: Java's `LiteActionOwner` and `FullActionOwner` (a memory optimization) are one struct.
#[derive(Clone)]
pub struct ActionOwner {
    label: Option<Label>,
    location: Location,
    target_kind: String,
    build_configuration_info: BuildConfigurationInfo,
    execution_platform: Option<PlatformInfoRef>,
    aspect_descriptors: Vec<AspectDescriptor>,
    exec_properties: IndexMap<String, String>,
}

impl ActionOwner {
    /// Creates a new `ActionOwner`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:56 ActionOwner.create
    pub fn create(
        label: Option<Label>,
        location: Location,
        target_kind: impl Into<String>,
        build_configuration_info: BuildConfigurationInfo,
        execution_platform: Option<PlatformInfoRef>,
        aspect_descriptors: Vec<AspectDescriptor>,
        exec_properties: IndexMap<String, String>,
    ) -> ActionOwner {
        ActionOwner {
            label,
            location,
            target_kind: target_kind.into(),
            build_configuration_info,
            execution_platform,
            aspect_descriptors,
            exec_properties,
        }
    }

    /// Creates a dummy `ActionOwner` for testing and for `SYSTEM_ACTION_OWNER`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:44 ActionOwner.createDummy
    pub fn create_dummy(
        label: Option<Label>,
        location: Location,
        target_kind: impl Into<String>,
        build_configuration_mnemonic: impl Into<String>,
        configuration_checksum: impl Into<String>,
        build_configuration_event: Option<BuildConfigurationEvent>,
        is_tool_configuration: bool,
        execution_platform: Option<PlatformInfoRef>,
        aspect_descriptors: Vec<AspectDescriptor>,
        exec_properties: IndexMap<String, String>,
    ) -> ActionOwner {
        ActionOwner::create(
            label,
            location,
            target_kind,
            BuildConfigurationInfo::create(
                build_configuration_mnemonic,
                configuration_checksum,
                build_configuration_event,
                is_tool_configuration,
            ),
            execution_platform,
            aspect_descriptors,
            exec_properties,
        )
    }

    /// Returns a user-friendly description of the owner, or `None`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:106 ActionOwner.getDescription
    pub fn get_description(&self) -> Option<String> {
        let label = self.label.as_ref()?;
        let target_description = format!("{} target {}", self.target_kind, label);
        if self.aspect_descriptors.is_empty() {
            return Some(target_description);
        }
        let aspect_names: Vec<&str> = self
            .aspect_descriptors
            .iter()
            .map(|a| a.description.as_str())
            .collect();
        Some(format!(
            "aspect{} [{}] on {}",
            if !self.aspect_descriptors.is_empty() {
                "s"
            } else {
                ""
            },
            aspect_names.join(", "),
            target_description
        ))
    }

    /// Returns the label for this `ActionOwner`, if any; `None` otherwise.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:107 ActionOwner.getLabel
    pub fn get_label(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    /// Returns the location for this `ActionOwner`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:133 ActionOwner.getLocation
    pub fn get_location(&self) -> &Location {
        &self.location
    }

    /// Returns the target kind (rule class name) for this `ActionOwner`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:111 ActionOwner.getTargetKind
    pub fn get_target_kind(&self) -> &str {
        &self.target_kind
    }

    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:139 ActionOwner.getBuildConfigurationInfo
    pub fn get_build_configuration_info(&self) -> &BuildConfigurationInfo {
        &self.build_configuration_info
    }

    /// Returns the mnemonic for the configuration for this `ActionOwner`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:142 ActionOwner.getBuildConfigurationMnemonic
    pub fn get_build_configuration_mnemonic(&self) -> &str {
        &self.build_configuration_info.mnemonic
    }

    /// Returns the short cache key for the configuration of this `ActionOwner`.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:152 ActionOwner.getConfigurationChecksum
    pub fn get_configuration_checksum(&self) -> &str {
        &self.build_configuration_info.checksum
    }

    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:161 ActionOwner.getBuildConfigurationEvent
    pub fn get_build_configuration_event(&self) -> Option<&BuildConfigurationEvent> {
        self.build_configuration_info
            .build_configuration_event
            .as_ref()
    }

    /// Returns whether the configuration for this `ActionOwner` is a tool configuration.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:169 ActionOwner.isBuildConfigurationForTool
    pub fn is_build_configuration_for_tool(&self) -> bool {
        self.build_configuration_info.is_tool_configuration
    }

    /// Returns the execution platform for this `ActionOwner`, if any.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:178 ActionOwner.getExecutionPlatform
    pub fn get_execution_platform(&self) -> Option<&PlatformInfoRef> {
        self.execution_platform.as_ref()
    }

    /// Returns the aspect descriptors, outermost first.
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:113 ActionOwner.getAspectDescriptors
    pub fn get_aspect_descriptors(&self) -> &[AspectDescriptor] {
        &self.aspect_descriptors
    }

    /// Returns the execution properties of this owner (from the exec group / target).
    // Java: com/google/devtools/build/lib/actions/ActionOwner.java:189 ActionOwner.getExecProperties
    pub fn get_exec_properties(&self) -> &IndexMap<String, String> {
        &self.exec_properties
    }
}

impl fmt::Debug for ActionOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionOwner")
            .field("label", &self.label)
            .field("location", &self.location)
            .field("target_kind", &self.target_kind)
            .field("build_configuration_info", &self.build_configuration_info)
            .field(
                "execution_platform",
                &self.execution_platform.as_ref().map(|p| p.label().clone()),
            )
            .field("aspect_descriptors", &self.aspect_descriptors)
            .field("exec_properties", &self.exec_properties)
            .finish()
    }
}
