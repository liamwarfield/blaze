//! The `PLATFORM` dependency of configuration creation: `PlatformValue.key(label, aliases)` and
//! the parts of `PlatformValue`/`PlatformInfo` configurations read.
//!
//! PORT: Java's `PlatformFunction` (analysis/platform) configures the platform target and reads
//! its `PlatformInfo`; that needs the analysis phase (CONFIGURED_TARGET of `platform()` and
//! `alias()` rules), which belongs to the W4 analysis work in `blaze_sky_analysis`. Until then the
//! function is a seam ([`PlatformValueProvider`]); [`OraclePlatforms`] serves facts recorded from
//! the oracle (`scripts/w4/oracle-platforms`), like W3's `OracleRepos` for external repositories.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_skyframe::{ComputeError, Environment, Key, SkyError, SkyFunctionName};

/// `PlatformValue.key(platformLabel, flagAliasMappings)`.
#[derive(Clone, PartialEq, Eq)]
pub struct PlatformValueKey {
    pub platform_label: Label,
    pub flag_alias_mappings: Vec<(String, Label)>,
}

impl Hash for PlatformValueKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.platform_label.hash(state);
        self.flag_alias_mappings.hash(state);
    }
}

impl fmt::Debug for PlatformValueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java: PlatformValue.Key is an AutoValue (record) printing its fields.
        write!(
            f,
            "Key[platformLabel={}, flagAliasMappings={{{}}}]",
            self.platform_label,
            self.flag_alias_mappings
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Key for PlatformValueKey {
    type Value = PlatformValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("PLATFORM");
}

/// The parts of `PlatformValue` (`platformInfo()`, `parsedFlags()`) configurations read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformValue {
    /// `platformInfo().label()`: the platform after alias resolution.
    pub label: Label,
    /// `platformInfo().constraints()`: constraint setting → constraint value.
    pub constraints: BTreeMap<Label, Label>,
    /// Whether `parsedFlags()` is present (the platform has `flags`).
    pub has_parsed_flags: bool,
}

/// `PlatformInfo` CPU constraint setting (`PlatformValue` users read
/// `constraints().get(CPU_CONSTRAINT_SETTING)`).
// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:34 ConstraintConstants.CPU_CONSTRAINT_SETTING
pub fn cpu_constraint_setting() -> Label {
    // PORT: Java parses "@platforms//cpu:cpu" with parseCanonicalUnchecked, which yields the
    // canonical repo name `platforms` (the same repository as @@platforms).
    Label::parse_canonical_unchecked("@platforms//cpu:cpu")
}

/// Computes `PlatformValue`s (Java `PlatformFunction`).
pub trait PlatformValueProvider: Send + Sync {
    fn compute(
        &self,
        key: &PlatformValueKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PlatformValue>, ComputeError>;

    /// The `InvalidPlatformException.class` of a lookup of a PLATFORM value
    /// (`tasks.lookUp(PlatformValue.key(..), InvalidPlatformException.class, ..)` of
    /// `PlatformProducer`, `env.getValueOrThrow(.., InvalidPlatformException.class)` of
    /// `BaselineOptionsFunction`): returns the exception of a failed dependency if it is an
    /// `InvalidPlatformException`, to be handled or rethrown by the caller.
    ///
    /// PORT: `InvalidPlatformException` is `blaze_analysis::toolchains::ToolchainException`
    /// (kind `InvalidPlatform`), which this crate cannot name; the provider that throws it
    /// recognizes it. A provider that never throws one keeps the default.
    // Java: com/google/devtools/build/lib/analysis/producers/PlatformProducer.java:71 PlatformProducer.acceptValueOrException2
    fn catch_invalid_platform_exception(&self, _exception: &SkyError) -> Option<SkyError> {
        None
    }
}

/// The default provider: the analysis phase is not ported.
pub struct UnportedPlatforms;

impl PlatformValueProvider for UnportedPlatforms {
    fn compute(
        &self,
        key: &PlatformValueKey,
        _env: &mut Environment<'_>,
    ) -> Result<Option<PlatformValue>, ComputeError> {
        blaze_util::unported!("PlatformFunction.compute", "{:?}", key)
    }
}

/// Platform facts recorded from the oracle (`scripts/w4/oracle-platforms`): a stand-in for
/// `PlatformFunction` until platform targets can be analyzed.
pub struct OraclePlatforms {
    platforms: BTreeMap<String, PlatformValue>,
}

impl OraclePlatforms {
    /// Parses the JSON `scripts/w4/oracle-platforms` writes:
    /// `{"platforms": {"<label>": {"label": "<resolved>", "constraints": {"<setting>":
    /// "<value>"}, "flags": [...]}}}`.
    pub fn parse(json: &str) -> Result<OraclePlatforms, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut platforms = BTreeMap::new();
        let entries = v
            .get("platforms")
            .and_then(|p| p.as_object())
            .ok_or("missing 'platforms'")?;
        for (label, p) in entries {
            let resolved = p
                .get("label")
                .and_then(|l| l.as_str())
                .ok_or("missing 'label'")?;
            let mut constraints = BTreeMap::new();
            if let Some(c) = p.get("constraints").and_then(|c| c.as_object()) {
                for (setting, value) in c {
                    constraints.insert(
                        Label::parse_canonical(setting).map_err(|e| e.get_message().to_string())?,
                        Label::parse_canonical(value.as_str().ok_or("constraint value")?)
                            .map_err(|e| e.get_message().to_string())?,
                    );
                }
            }
            let has_parsed_flags = p
                .get("flags")
                .and_then(|f| f.as_array())
                .is_some_and(|f| !f.is_empty());
            platforms.insert(
                Label::parse_canonical(label)
                    .map_err(|e| e.get_message().to_string())?
                    .get_unambiguous_canonical_form(),
                PlatformValue {
                    label: Label::parse_canonical(resolved)
                        .map_err(|e| e.get_message().to_string())?,
                    constraints,
                    has_parsed_flags,
                },
            );
        }
        Ok(OraclePlatforms { platforms })
    }

    /// The recorded value of a platform label.
    pub fn get(&self, label: &Label) -> Option<&PlatformValue> {
        self.platforms.get(&label.get_unambiguous_canonical_form())
    }

    pub fn load(path: &std::path::Path) -> Result<OraclePlatforms, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        Self::parse(&text)
    }
}

impl PlatformValueProvider for OraclePlatforms {
    fn compute(
        &self,
        key: &PlatformValueKey,
        _env: &mut Environment<'_>,
    ) -> Result<Option<PlatformValue>, ComputeError> {
        match self
            .platforms
            .get(&key.platform_label.get_unambiguous_canonical_form())
        {
            Some(v) => {
                if v.has_parsed_flags {
                    blaze_util::unported!(
                        "PlatformFunction.compute",
                        "platform flags of {}",
                        key.platform_label
                    );
                }
                Ok(Some(v.clone()))
            }
            None => blaze_util::unported!(
                "PlatformFunction.compute",
                "no recorded facts for platform {}",
                key.platform_label
            ),
        }
    }
}

/// Shared provider handle.
pub type SharedPlatformValueProvider = Arc<dyn PlatformValueProvider>;
