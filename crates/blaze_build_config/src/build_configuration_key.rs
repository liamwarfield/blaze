//! Port of `skyframe/config/BuildConfigurationKey.java` and
//! `skyframe/config/BuildConfigurationKeyValue.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;

use crate::build_options::{BuildOptions, ScopeType};

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

    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:49 BuildConfigurationKey.getOptions
    pub fn get_options(&self) -> &Arc<BuildOptions> {
        &self.options
    }

    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:58 BuildConfigurationKey.getOptionsChecksum
    pub fn get_options_checksum(&self) -> &str {
        self.options.checksum()
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:63 BuildConfigurationKey.equals
impl PartialEq for BuildConfigurationKey {
    fn eq(&self, other: &Self) -> bool {
        *self.options == *other.options
    }
}
impl Eq for BuildConfigurationKey {}
// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKey.java:74 BuildConfigurationKey.hashCode
// PORT: skipped, key interning and the serialization codec (remote analysis caching, not ported): BuildConfigurationKey.getSkyKeyInterner, BuildConfigurationKey.codec, BuildConfigurationKey.CodecHolder.<init>
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
    /// PORT: the `label` of a `BuildConfigurationKeyProducer` started by
    /// `BuildConfigurationKeyMapProducer` (the target whose transition produced the options).
    /// Java runs that producer inside the analysis state machine; Blaze maps the options through
    /// this node, so the label is part of the request. `BuildConfigurationKeyFunction` passes
    /// null (`create`). The producer reads the label only to reset project-scoped flags
    /// (`resetFlags`), so `create_for_producer` keeps it only when the options have a Starlark
    /// flag whose scope is PROJECT or not known yet; other requests share the label-less node.
    producer_label: Option<Label>,
}

impl BuildConfigurationKeyValueKey {
    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:37 BuildConfigurationKeyValue.Key.create
    pub fn create(build_options: Arc<BuildOptions>) -> Self {
        BuildConfigurationKeyValueKey {
            build_options,
            producer_label: None,
        }
    }

    /// The request of a `BuildConfigurationKeyProducer` of `BuildConfigurationKeyMapProducer`
    /// for the target `label` (see `producer_label`).
    pub fn create_for_producer(build_options: Arc<BuildOptions>, label: Option<&Label>) -> Self {
        // Java: com/google/devtools/build/lib/analysis/producers/BuildConfigurationKeyProducer.java:170 BuildConfigurationKeyProducer.findBuildOptionsScopes
        // (the condition under which the scope lookup, and so resetFlags, can happen).
        let may_reset_flags = build_options.get_starlark_options().keys().any(|key| {
            let scope_type = build_options.get_scope_type_map().get(key);
            scope_type.is_none() || scope_type == Some(&ScopeType::PROJECT)
        });
        BuildConfigurationKeyValueKey {
            build_options,
            producer_label: if may_reset_flags {
                label.cloned()
            } else {
                None
            },
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:53 BuildConfigurationKeyValue.Key.buildOptions
    pub fn build_options(&self) -> &Arc<BuildOptions> {
        &self.build_options
    }

    pub fn producer_label(&self) -> Option<&Label> {
        self.producer_label.as_ref()
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:63 BuildConfigurationKeyValue.Key.equals
// PORT: Java compares the options only; the producer label is part of the request here (see
// `producer_label`).
impl PartialEq for BuildConfigurationKeyValueKey {
    fn eq(&self, other: &Self) -> bool {
        *self.build_options == *other.build_options && self.producer_label == other.producer_label
    }
}
impl Eq for BuildConfigurationKeyValueKey {}
// Java: com/google/devtools/build/lib/skyframe/config/BuildConfigurationKeyValue.java:75 BuildConfigurationKeyValue.Key.hashCode
// PORT: skipped, key interning (the keys are plain values here): BuildConfigurationKeyValue.Key.intern, BuildConfigurationKeyValue.Key.getSkyKeyInterner
impl Hash for BuildConfigurationKeyValueKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.build_options.hash(state);
        self.producer_label.hash(state);
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
