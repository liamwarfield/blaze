//! The parts of `analysis/platform/PlatformInfo` the action layer reads.
//!
//! PORT: `PlatformInfo` is a provider owned by the analysis layer (`analysis/platform`). Actions
//! only read its label (aquery `executionPlatform`) and fingerprint it into every action key
//! (`ActionKeyComputer.getKey`). The analysis crate implements this trait for its real
//! `PlatformInfo`; [`PlatformInfoData`] is a plain-data port of the fingerprinted fields for
//! tests and for callers that already hold the values.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_util::Fingerprint;
use indexmap::IndexMap;

/// The members of `PlatformInfo` used by actions.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:41 PlatformInfo
pub trait PlatformInfo: Send + Sync + fmt::Debug {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:121 PlatformInfo.label
    fn label(&self) -> &Label;

    /// Add this platform to the given fingerprint.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:165 PlatformInfo.addTo
    fn add_to(&self, fp: &mut Fingerprint);

    /// `execProperties().properties()`.
    fn exec_properties(&self) -> IndexMap<String, String>;
}

/// Shared handle.
pub type PlatformInfoRef = Arc<dyn PlatformInfo>;

/// One `ConstraintValueInfo` as fingerprinted: its setting's label and its own label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintValueData {
    pub setting: Label,
    pub value: Label,
}

/// Plain-data `PlatformInfo` (the fingerprinted fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformInfoData {
    pub label: Label,
    /// `ConstraintCollection`: the parent (if any) and the constraints in map order.
    pub constraints_parent: Option<Box<ConstraintCollectionData>>,
    pub constraints: Vec<ConstraintValueData>,
    pub remote_execution_properties: String,
    pub exec_properties: IndexMap<String, String>,
    pub flags: Vec<String>,
    pub required_settings: Vec<Label>,
    pub allowed_toolchain_types: Vec<Label>,
    pub check_toolchain_types: bool,
    pub missing_toolchain_error_message: Option<String>,
}

/// Plain-data `ConstraintCollection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintCollectionData {
    pub parent: Option<Box<ConstraintCollectionData>>,
    pub constraints: Vec<ConstraintValueData>,
}

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:283 ConstraintCollection.addToFingerprint
fn constraints_add_to_fingerprint(
    parent: Option<&ConstraintCollectionData>,
    constraints: &[ConstraintValueData],
    fp: &mut Fingerprint,
) {
    // Encode whether there is a parent.
    fp.add_boolean(parent.is_some());
    // Add the parent.
    if let Some(p) = parent {
        constraints_add_to_fingerprint(p.parent.as_deref(), &p.constraints, fp);
    }
    // Add the actual constraints.
    fp.add_int(constraints.len() as i32);
    for c in constraints {
        // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:107 ConstraintValueInfo.addTo
        // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:96 ConstraintSettingInfo.addTo
        fp.add_string(&c.setting.get_canonical_form());
        fp.add_string(&c.value.get_canonical_form());
    }
}

impl PlatformInfo for PlatformInfoData {
    fn label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:165 PlatformInfo.addTo
    fn add_to(&self, fp: &mut Fingerprint) {
        // Java: label.toString() is the canonical form.
        fp.add_string(&self.label.get_canonical_form());
        constraints_add_to_fingerprint(self.constraints_parent.as_deref(), &self.constraints, fp);
        fp.add_nullable_string(Some(&self.remote_execution_properties));
        fp.add_string_map(
            self.exec_properties
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        fp.add_strings(self.flags.iter());
        let required: Vec<String> = self
            .required_settings
            .iter()
            .map(|l| l.get_canonical_form())
            .collect();
        fp.add_strings(required.iter());
        let allowed: Vec<String> = self
            .allowed_toolchain_types
            .iter()
            .map(|l| l.get_canonical_form())
            .collect();
        fp.add_strings(allowed.iter());
        fp.add_boolean(self.check_toolchain_types);
        fp.add_nullable_string(self.missing_toolchain_error_message.as_deref());
    }

    fn exec_properties(&self) -> IndexMap<String, String> {
        self.exec_properties.clone()
    }
}

/// `PlatformConstants.INTERNAL_PLATFORM`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformConstants.java:23 PlatformConstants.INTERNAL_PLATFORM
pub fn internal_platform() -> Label {
    Label::parse_canonical_unchecked("@bazel_tools//tools:internal_platform")
}

/// `PlatformInfo.EMPTY_PLATFORM_INFO`: `PlatformInfo.builder().setLabel(INTERNAL_PLATFORM).build()`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:58 PlatformInfo.EMPTY_PLATFORM_INFO
pub fn empty_platform_info() -> PlatformInfoRef {
    static EMPTY: std::sync::LazyLock<PlatformInfoRef> = std::sync::LazyLock::new(|| {
        Arc::new(PlatformInfoData {
            label: internal_platform(),
            constraints_parent: None,
            constraints: Vec::new(),
            // Builder.mergeRemoteExecutionProperties(null parent, null) returns "".
            remote_execution_properties: String::new(),
            exec_properties: IndexMap::new(),
            flags: Vec::new(),
            required_settings: Vec::new(),
            allowed_toolchain_types: Vec::new(),
            check_toolchain_types: false,
            missing_toolchain_error_message: None,
        })
    });
    Arc::clone(&EMPTY)
}
