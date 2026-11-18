//! Port of `analysis/platform/ConstraintSettingInfo.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::platform_common::CONSTRAINT_SETTING_INFO;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StructureMarker;
use blaze_util::Fingerprint;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use crate::platform::constraint_collection::{gated_get_attr, gated_has_attr};
use starlark::collections::Hashed;
use starlark::eval::Evaluator;
use starlark::values::ValueLike;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::info::InfoProviderKey;
use crate::platform::constraint_value_info::ConstraintValueInfo;

/// Provider for a platform constraint setting that is available to be fulfilled.
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:30 ConstraintSettingInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConstraintSettingInfo {
    #[allocative(skip)]
    label: Label,
    #[allocative(skip)]
    default_constraint_value_label: Option<Label>,
    #[allocative(skip)]
    refined_constraint_value: Option<Arc<ConstraintValueInfo>>,
}

starlark_simple_value!(ConstraintSettingInfo);

impl ConstraintSettingInfo {
    /// Name used in Starlark for accessing this provider.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:32 ConstraintSettingInfo.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "ConstraintSettingInfo";

    /// Java `PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        CONSTRAINT_SETTING_INFO.key()
    }

    /// Returns a new [`ConstraintSettingInfo`] with the given data.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:127 ConstraintSettingInfo.create
    pub fn create(constraint_setting: Label) -> ConstraintSettingInfo {
        Self::create_with(constraint_setting, None, None)
    }

    /// Returns a new [`ConstraintSettingInfo`] with the given data.
    // PORT: Java overload create(Label, Label, ConstraintValueInfo).
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:133 ConstraintSettingInfo.create
    pub fn create_with(
        constraint_setting: Label,
        default_constraint_value: Option<Label>,
        refined_constraint_value: Option<ConstraintValueInfo>,
    ) -> ConstraintSettingInfo {
        ConstraintSettingInfo {
            label: constraint_setting,
            default_constraint_value_label: default_constraint_value,
            refined_constraint_value: refined_constraint_value.map(Arc::new),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:57 ConstraintSettingInfo.label
    pub fn label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:62 ConstraintSettingInfo.hasDefaultConstraintValue
    pub fn has_default_constraint_value(&self) -> bool {
        self.default_constraint_value_label.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:68 ConstraintSettingInfo.defaultConstraintValue
    pub fn default_constraint_value(&self) -> Option<ConstraintValueInfo> {
        let label = self.default_constraint_value_label.as_ref()?;
        Some(ConstraintValueInfo::create(self.clone(), label.clone()))
    }

    /// Returns the refined constraint value, or `None` if not set.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:77 ConstraintSettingInfo.refinedConstraintValue
    pub fn refined_constraint_value(&self) -> Option<&ConstraintValueInfo> {
        self.refined_constraint_value.as_deref()
    }

    /// Iterates the constraint value labels that any `constraint_value` for this setting
    /// recursively implies via the `refines_constraint_value` chain.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:85 ConstraintSettingInfo.refinementChain
    pub fn refinement_chain(&self) -> Vec<Label> {
        let mut result = Vec::new();
        let mut cv = self.refined_constraint_value.as_deref();
        while let Some(v) = cv {
            result.push(v.label().clone());
            cv = v.constraint().refined_constraint_value.as_deref();
        }
        result
    }

    /// Add this constraint setting to the given fingerprint.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:96 ConstraintSettingInfo.addTo
    pub fn add_to(&self, fp: &mut Fingerprint) {
        fp.add_string(&self.label.get_canonical_form());
    }
}

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:101 ConstraintSettingInfo.equals
impl PartialEq for ConstraintSettingInfo {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
    }
}

impl Eq for ConstraintSettingInfo {}

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:110 ConstraintSettingInfo.hashCode
impl Hash for ConstraintSettingInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.label.hash(state);
    }
}

impl fmt::Display for ConstraintSettingInfo {
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:115 ConstraintSettingInfo.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConstraintSettingInfo({}", self.label)?;
        if let Some(d) = &self.default_constraint_value_label {
            write!(f, ", default_constraint_value={}", d)?;
        }
        if let Some(r) = &self.refined_constraint_value {
            write!(f, ", refines_constraint_value={}", r.label())?;
        }
        write!(f, ")")
    }
}

/// The struct fields of `ConstraintSettingInfoApi`, sorted.
/// The fields with `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API`
/// (ConstraintSettingInfoApi.java:36, :43): absent when the flag is off.
const GATED: [&str; 2] = ["default_constraint_value", "label"];
const FIELDS: [&str; 3] = [
    "default_constraint_value",
    "has_default_constraint_value",
    "label",
];

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/ConstraintSettingInfoApi.java:33 ConstraintSettingInfoApi
#[starlark_value(type = "ConstraintSettingInfo")]
impl<'v> StarlarkValue<'v> for ConstraintSettingInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "label" => Some(heap.alloc(LabelValue(self.label.clone()))),
            "default_constraint_value" => Some(match self.default_constraint_value() {
                Some(v) => heap.alloc(v),
                None => Value::new_none(),
            }),
            "has_default_constraint_value" => {
                Some(Value::new_bool(self.has_default_constraint_value()))
            }
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
            .downcast_ref::<ConstraintSettingInfo>()
            .is_some_and(|o| o == self))
    }

    // Java: NativeInfo.isImmutable is true (hashable).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.label.get_canonical_form().hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(CONSTRAINT_SETTING_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfoTest.java constraintSetting_equalsTester
    #[test]
    fn constraint_setting_equals_tester() {
        let a = ConstraintSettingInfo::create(label("//constraint:basic"));
        let b = ConstraintSettingInfo::create(label("//constraint:basic"));
        let c = ConstraintSettingInfo::create(label("//constraint:other"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn repr() {
        let s = ConstraintSettingInfo::create_with(
            label("//constraint:basic"),
            Some(label("//constraint:default")),
            None,
        );
        assert_eq!(
            s.to_string(),
            "ConstraintSettingInfo(//constraint:basic, default_constraint_value=//constraint:default)"
        );
    }
}
