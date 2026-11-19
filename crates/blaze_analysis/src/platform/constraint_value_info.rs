//! Port of `analysis/platform/ConstraintValueInfo.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::platform::constraint_collection::{gated_get_attr, gated_has_attr, no_field_message};
use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::platform_common::CONSTRAINT_VALUE_INFO;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StructureMarker;
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::collections::Hashed;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::config::config_matching_provider::{ConfigMatchingProvider, Diff, MatchResult};
use crate::info::InfoProviderKey;
use crate::platform::constraint_setting_info::ConstraintSettingInfo;
use crate::platform::platform_info::PlatformInfo;

/// Provider for a platform constraint value that fulfills a [`ConstraintSettingInfo`].
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:33 ConstraintValueInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConstraintValueInfo {
    #[allocative(skip)]
    constraint: Arc<ConstraintSettingInfo>,
    #[allocative(skip)]
    label: Label,
}

starlark_simple_value!(ConstraintValueInfo);

impl ConstraintValueInfo {
    /// Name used in Starlark for accessing this provider.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:35 ConstraintValueInfo.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "ConstraintValueInfo";

    /// Java `PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        CONSTRAINT_VALUE_INFO.key()
    }

    /// Returns a new [`ConstraintValueInfo`] with the given data.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:102 ConstraintValueInfo.create
    pub fn create(constraint: ConstraintSettingInfo, value: Label) -> ConstraintValueInfo {
        ConstraintValueInfo {
            constraint: Arc::new(constraint),
            label: value,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:55 ConstraintValueInfo.constraint
    pub fn constraint(&self) -> &ConstraintSettingInfo {
        &self.constraint
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:60 ConstraintValueInfo.label
    pub fn label(&self) -> &Label {
        &self.label
    }

    /// Returns a `ConfigMatchingProvider` that matches if the owning target's platform includes
    /// this constraint.
    ///
    /// The `ConstraintValue` rule can't directly return a `ConfigMatchingProvider` because, as
    /// part of a platform's definition, it doesn't have access to the platform during its
    /// analysis.
    ///
    /// Instead, a target with a `select()` on a `ConstraintValue` passes its platform info to
    /// this method.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:76 ConstraintValueInfo.configMatchingProvider
    pub fn config_matching_provider(&self, platform_info: &PlatformInfo) -> ConfigMatchingProvider {
        let platform_value = platform_info.constraints().get(self.constraint());
        ConfigMatchingProvider::create(
            self.label.clone(),
            Vec::new(),
            IndexMap::new(),
            IndexSet::new(),
            self.compute_match_result(platform_value.as_ref()),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:86 ConstraintValueInfo.computeMatchResult
    fn compute_match_result(&self, platform_value: Option<&ConstraintValueInfo>) -> MatchResult {
        if platform_value == Some(self) {
            MatchResult::Match
        } else {
            MatchResult::NoMatch(vec![Diff {
                what: self.constraint().label().clone(),
                want: self.label().get_name().to_owned(),
                got: match platform_value {
                    Some(v) => v.label().get_name().to_owned(),
                    None => "<unset>".to_owned(),
                },
            }])
        }
    }

    /// Add this constraint value to the given fingerprint.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:107 ConstraintValueInfo.addTo
    pub fn add_to(&self, fp: &mut Fingerprint) {
        self.constraint.add_to(fp);
        fp.add_string(&self.label.get_canonical_form());
    }
}

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:113 ConstraintValueInfo.equals
impl PartialEq for ConstraintValueInfo {
    fn eq(&self, other: &Self) -> bool {
        *self.constraint == *other.constraint && self.label == other.label
    }
}

impl Eq for ConstraintValueInfo {}

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:121 ConstraintValueInfo.hashCode
impl Hash for ConstraintValueInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.constraint.hash(state);
        self.label.hash(state);
    }
}

impl fmt::Display for ConstraintValueInfo {
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:97 ConstraintValueInfo.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ConstraintValueInfo(setting={}, {})",
            self.constraint.label(),
            self.label
        )
    }
}

/// The struct fields of `ConstraintValueInfoApi`, sorted.
const FIELDS: [&str; 2] = ["constraint", "label"];
/// The fields with `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API`
/// (ConstraintValueInfoApi.java:37, :46): absent when the flag is off.
const GATED: [&str; 2] = ["constraint", "label"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/ConstraintValueInfoApi.java:33 ConstraintValueInfoApi
#[starlark_value(type = "ConstraintValueInfo")]
impl<'v> StarlarkValue<'v> for ConstraintValueInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "constraint" => Some(heap.alloc((*self.constraint).clone())),
            "label" => Some(heap.alloc(LabelValue(self.label.clone()))),
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

    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(no_field_message("ConstraintValueInfo", attribute, &FIELDS))
    }

    // PORT: Java's `dir` leaves out the GATED fields when the flag is off
    // (CallUtils.getAnnotatedMethods); `dir_attr` has no evaluator, so they are listed.

    fn dir_attr(&self) -> Vec<String> {
        FIELDS.iter().map(|s| (*s).to_owned()).collect()
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ConstraintValueInfo>()
            .is_some_and(|o| o == self))
    }

    // Java: NativeInfo.isImmutable is true (hashable).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.constraint.label().get_canonical_form().hash(hasher);
        self.label.get_canonical_form().hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(CONSTRAINT_VALUE_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfoTest.java constraintValue_equalsTester
    #[test]
    fn constraint_value_equals_tester() {
        let setting1 = ConstraintSettingInfo::create(label("//constraint:basic"));
        let setting2 = ConstraintSettingInfo::create(label("//constraint:other"));
        // Base case.
        let a = ConstraintValueInfo::create(setting1.clone(), label("//constraint:value"));
        let b = ConstraintValueInfo::create(setting1.clone(), label("//constraint:value"));
        // Different label.
        let c = ConstraintValueInfo::create(setting1, label("//constraint:otherValue"));
        // Different setting.
        let d = ConstraintValueInfo::create(setting2, label("//constraint:otherValue"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_ne!(c, d);
    }

    #[test]
    fn repr() {
        let setting = ConstraintSettingInfo::create(label("//constraint:basic"));
        let v = ConstraintValueInfo::create(setting, label("//constraint:value"));
        assert_eq!(
            v.to_string(),
            "ConstraintValueInfo(setting=//constraint:basic, //constraint:value)"
        );
    }
}
