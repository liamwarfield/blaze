//! Port of `skyframe/config/BuildConfigurationKey.java` and
//! `skyframe/config/BuildConfigurationKeyValue.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::build_options::BuildOptions;

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:29 BuildConfigurationKey
/// `SkyKey` for `BuildConfigurationValue`.
#[derive(Clone)]
pub struct BuildConfigurationKey {
    options: Arc<BuildOptions>,
}

impl BuildConfigurationKey {
    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:39 BuildConfigurationKey.create
    pub fn create(options: Arc<BuildOptions>) -> Self {
        BuildConfigurationKey { options }
    }

    pub fn get_options(&self) -> &Arc<BuildOptions> {
        &self.options
    }

    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:58 BuildConfigurationKey.getOptionsChecksum
    pub fn get_options_checksum(&self) -> &str {
        self.options.checksum()
    }
}

impl PartialEq for BuildConfigurationKey {
    fn eq(&self, other: &Self) -> bool {
        *self.options == *other.options
    }
}
impl Eq for BuildConfigurationKey {}
impl Hash for BuildConfigurationKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.options.hash(state)
    }
}

impl fmt::Debug for BuildConfigurationKey {
    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:79 BuildConfigurationKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // This format is depended on by integration tests.
        write!(f, "BuildConfigurationKey[{}]", self.options.checksum())
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:34 BuildConfigurationKeyValue.Key
/// `SkyKey` for `BuildConfigurationKeyValue`: the options before platform mappings and
/// platform-based flags are applied.
#[derive(Clone)]
pub struct BuildConfigurationKeyValueKey {
    build_options: Arc<BuildOptions>,
}

impl BuildConfigurationKeyValueKey {
    pub fn create(build_options: Arc<BuildOptions>) -> Self {
        BuildConfigurationKeyValueKey { build_options }
    }

    pub fn build_options(&self) -> &Arc<BuildOptions> {
        &self.build_options
    }
}

impl PartialEq for BuildConfigurationKeyValueKey {
    fn eq(&self, other: &Self) -> bool {
        *self.build_options == *other.build_options
    }
}
impl Eq for BuildConfigurationKeyValueKey {}
impl Hash for BuildConfigurationKeyValueKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.build_options.hash(state)
    }
}

impl fmt::Debug for BuildConfigurationKeyValueKey {
    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:80 Key.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildConfigurationKeyValue.Key{{buildOptions={}}}",
            self.build_options.checksum()
        )
    }
}
