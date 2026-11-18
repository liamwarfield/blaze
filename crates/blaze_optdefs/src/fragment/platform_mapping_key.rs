//! Port of `skyframe/config/PlatformMappingKey.java` (the value side; the Skyframe key plumbing
//! belongs to the configuration crate).

use blaze_options::{OptionScalar, option_field_type};
use blaze_vfs::PathFragment;

// Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:29 PlatformMappingKey
/// Key for `PlatformMappingValue` based on the location of the mapping file.
// PORT: skipped, incremental (SkyKeyInterner interning; equality is by value).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlatformMappingKey {
    path: PathFragment,
    was_explicitly_set_by_user: bool,
}

impl PlatformMappingKey {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:34 PlatformMappingKey.DEFAULT
    /// Default key to use when the user does not explicitly set `--platform_mappings`.
    pub fn default_key() -> PlatformMappingKey {
        Self::create(PathFragment::create("platform_mappings"), false)
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:41 PlatformMappingKey.createExplicitlySet
    /// Creates a platform mapping key with the given, main workspace-relative path to the
    /// mappings file which was specified by the user via the `--platform_mappings` flag.
    pub fn create_explicitly_set(workspace_relative_mapping_path: PathFragment) -> Self {
        Self::create(workspace_relative_mapping_path, true)
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:45 PlatformMappingKey.create
    fn create(
        workspace_relative_mapping_path: PathFragment,
        was_explicitly_set_by_user: bool,
    ) -> Self {
        PlatformMappingKey {
            path: workspace_relative_mapping_path,
            was_explicitly_set_by_user,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:66 PlatformMappingKey.getWorkspaceRelativeMappingPath
    /// Returns the main-workspace relative path this mapping's mapping file can be found at.
    pub fn get_workspace_relative_mapping_path(&self) -> &PathFragment {
        &self.path
    }

    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:70 PlatformMappingKey.wasExplicitlySetByUser
    pub fn was_explicitly_set_by_user(&self) -> bool {
        self.was_explicitly_set_by_user
    }
}

impl OptionScalar for PlatformMappingKey {
    // Java: com/google/devtools/build/lib/skyframe/config/PlatformMappingKey.java:96 PlatformMappingKey.toString
    fn java_to_string(&self) -> String {
        format!(
            "PlatformMappingKey{{path={}, wasExplicitlySetByUser={}}}",
            self.path.get_path_string(),
            self.was_explicitly_set_by_user
        )
    }
}

option_field_type!(
    PlatformMappingKey,
    "com.google.devtools.build.lib.skyframe.config.PlatformMappingKey",
    raw = PlatformMappingKey::default_key()
);
