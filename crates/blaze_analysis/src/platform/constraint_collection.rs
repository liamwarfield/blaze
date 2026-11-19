//! Port of `analysis/platform/ConstraintCollection.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use allocative::Allocative;
use blaze_starlark::ThreadContext;
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::collections::Hashed;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::platform::constraint_setting_info::ConstraintSettingInfo;
use crate::platform::constraint_value_info::ConstraintValueInfo;

/// A collection of constraint values.
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:52 ConstraintCollection
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConstraintCollection {
    #[allocative(skip)]
    parent: Option<Arc<ConstraintCollection>>,
    #[allocative(skip)]
    constraints: Arc<IndexMap<ConstraintSettingInfo, ConstraintValueInfo>>,
}

starlark_simple_value!(ConstraintCollection);

/// A builder class to help create instances of [`ConstraintCollection`].
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:56 ConstraintCollection.Builder
#[derive(Clone, Debug, Default)]
pub struct ConstraintCollectionBuilder {
    parent: Option<Arc<ConstraintCollection>>,
    constraint_values: Vec<ConstraintValueInfo>,
}

impl ConstraintCollectionBuilder {
    /// Sets the parent [`ConstraintCollection`] of this instance.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:65 ConstraintCollection.Builder.parent
    pub fn parent(&mut self, parent: Option<ConstraintCollection>) -> &mut Self {
        self.parent = parent.map(Arc::new);
        self
    }

    /// Adds the given constraints to the current collection.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:77 ConstraintCollection.Builder.addConstraints
    pub fn add_constraints(
        &mut self,
        constraints: impl IntoIterator<Item = ConstraintValueInfo>,
    ) -> &mut Self {
        self.constraint_values.extend(constraints);
        self
    }

    /// Returns the completed [`ConstraintCollection`] instance.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:83 ConstraintCollection.Builder.build
    pub fn build(&self) -> Result<ConstraintCollection, DuplicateConstraintException> {
        let constraint_values = self.constraint_values.clone();
        ConstraintCollection::validate_constraints(&constraint_values)?;
        // Duplicates have been removed by validateConstraints, so the first value for each
        // setting is used (toImmutableMap with a merge function keeping the first).
        let mut constraints = IndexMap::new();
        for v in constraint_values {
            constraints.entry(v.constraint().clone()).or_insert(v);
        }
        Ok(ConstraintCollection {
            parent: self.parent.clone(),
            constraints: Arc::new(constraints),
        })
    }
}

impl ConstraintCollection {
    /// Returns a new builder suitable for creating [`ConstraintCollection`] instances.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:101 ConstraintCollection.builder
    pub fn builder() -> ConstraintCollectionBuilder {
        ConstraintCollectionBuilder::default()
    }

    /// Returns the parent [`ConstraintCollection`] for this instance, or `None` if none exists.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:115 ConstraintCollection.parent
    pub fn parent(&self) -> Option<&ConstraintCollection> {
        self.parent.as_deref()
    }

    /// Returns the constraints supplied by this collection.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:118 ConstraintCollection.constraints
    pub fn constraints(&self) -> &IndexMap<ConstraintSettingInfo, ConstraintValueInfo> {
        &self.constraints
    }

    /// Returns `true` if this collection contains every [`ConstraintValueInfo`] in `expected`,
    /// or if the expected constraint value is the default for its setting.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:125 ConstraintCollection.containsAll
    pub fn contains_all<'a>(
        &self,
        expected: impl IntoIterator<Item = &'a ConstraintValueInfo>,
    ) -> bool {
        self.find_missing(expected).is_empty()
    }

    /// Returns the constraints from `expected` that are not present in this collection, either
    /// directly, or by being the default for their [`ConstraintSettingInfo`].
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:134 ConstraintCollection.findMissing
    pub fn find_missing<'a>(
        &self,
        expected: impl IntoIterator<Item = &'a ConstraintValueInfo>,
    ) -> Vec<ConstraintValueInfo> {
        let mut missing = Vec::new();
        for constraint in expected {
            let setting = constraint.constraint();
            let target_value = self.get(setting);
            if target_value.as_ref() != Some(constraint) {
                missing.push(constraint.clone());
            }
        }
        missing
    }

    /// Returns the settings where this collection and `other` have different values.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:151 ConstraintCollection.diff
    pub fn diff(&self, other: &ConstraintCollection) -> IndexSet<ConstraintSettingInfo> {
        let mut union = self.all_constraint_settings();
        union.extend(other.all_constraint_settings());
        union
            .into_iter()
            .filter(|setting| self.get(setting) != other.get(setting))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:157 ConstraintCollection.convertKey
    fn convert_key<'v>(key: Value<'v>) -> starlark::Result<ConstraintSettingInfo> {
        match key.downcast_ref::<ConstraintSettingInfo>() {
            Some(s) => Ok(s.clone()),
            None => Err(errorf(format!(
                "Constraint names must be platform_common.ConstraintSettingInfo, got {} instead",
                key.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:168 ConstraintCollection.has
    pub fn has(&self, constraint: &ConstraintSettingInfo) -> bool {
        // First, check locally.
        if self.constraints.contains_key(constraint) {
            return true;
        }

        // Then, check the parent, directly to ignore defaults.
        if let Some(parent) = self.parent() {
            return parent.has(constraint);
        }

        constraint.has_default_constraint_value()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:182 ConstraintCollection.hasWithoutDefault
    pub fn has_without_default(&self, constraint: &ConstraintSettingInfo) -> bool {
        // First, check locally.
        if self.constraints.contains_key(constraint) {
            return true;
        }

        // Then, check the parent, directly to ignore defaults.
        if let Some(parent) = self.parent() {
            return parent.has_without_default(constraint);
        }

        false
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:197 ConstraintCollection.hasConstraintValue
    pub fn has_constraint_value(&self, constraint_value: &ConstraintValueInfo) -> bool {
        let discovered_constraint_value = self.get(constraint_value.constraint());
        discovered_constraint_value.as_ref() == Some(constraint_value)
    }

    /// Returns the [`ConstraintValueInfo`] for the given setting, or `None` if none exists.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:208 ConstraintCollection.get
    pub fn get(&self, constraint: &ConstraintSettingInfo) -> Option<ConstraintValueInfo> {
        // First, check locally.
        if let Some(v) = self.constraints.get(constraint) {
            return Some(v.clone());
        }

        // Then, check the parent, directly to ignore defaults.
        if let Some(parent) = self.parent() {
            return parent.get(constraint);
        }

        // Finally, since this has no parent, check for a default.
        constraint.default_constraint_value()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:224 ConstraintCollection.constraintSettings
    pub fn constraint_settings(&self) -> Vec<ConstraintSettingInfo> {
        self.constraints.keys().cloned().collect()
    }

    /// Returns the settings set by this collection or any of its parents, without considering
    /// defaults.
    // PORT: public (Java private) so PlatformFunction can fill the configuration seam's
    // PlatformValue.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:232 ConstraintCollection.allConstraintSettings
    pub fn all_constraint_settings(&self) -> IndexSet<ConstraintSettingInfo> {
        let mut settings = IndexSet::new();
        let mut current = Some(self);
        while let Some(c) = current {
            settings.extend(c.constraints.keys().cloned());
            current = c.parent();
        }
        settings
    }

    /// Adds information to the fingerprint to uniquely identify this collection of constraints.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:283 ConstraintCollection.addToFingerprint
    pub fn add_to_fingerprint(&self, fp: &mut Fingerprint) {
        // Add the parent.
        fp.add_boolean(self.parent.is_some());
        if let Some(parent) = self.parent() {
            parent.add_to_fingerprint(fp);
        }
        // Add the constraints.
        fp.add_int(self.constraints.len() as i32);
        for constraint_value in self.constraints.values() {
            constraint_value.add_to(fp);
        }
    }

    /// Validates that the given constraints do not contain conflicting values: no setting has
    /// multiple different values. Multiple instances of the same constraint value are allowed.
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:305 ConstraintCollection.validateConstraints
    pub fn validate_constraints<'a>(
        constraint_values: impl IntoIterator<Item = &'a ConstraintValueInfo>,
    ) -> Result<(), DuplicateConstraintException> {
        // Collect the constraints by the settings.
        let mut constraints: IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>> =
            IndexMap::new();
        for v in constraint_values {
            constraints
                .entry(v.constraint().clone())
                .or_default()
                .push(v.clone());
        }

        // Find settings with duplicate values (distinct values, keeping first occurrences).
        let mut duplicates: IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>> =
            IndexMap::new();
        for (setting, values) in constraints {
            let distinct: IndexSet<ConstraintValueInfo> = values.into_iter().collect();
            if distinct.len() > 1 {
                duplicates.insert(setting, distinct.into_iter().collect());
            }
        }

        if !duplicates.is_empty() {
            return Err(DuplicateConstraintException::new(duplicates));
        }
        Ok(())
    }
}

// Java: AutoValue equals of ConstraintCollection (parent, constraints).
impl PartialEq for ConstraintCollection {
    fn eq(&self, other: &Self) -> bool {
        self.parent == other.parent && *self.constraints == *other.constraints
    }
}

impl Eq for ConstraintCollection {}

impl Hash for ConstraintCollection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parent.hash(state);
        for (k, v) in self.constraints.iter() {
            k.hash(state);
            v.hash(state);
        }
    }
}

impl fmt::Display for ConstraintCollection {
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:262 ConstraintCollection.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<")?;
        if let Some(parent) = self.parent() {
            write!(f, "parent: {}, ", parent)?;
        }
        write!(f, "[")?;
        write!(
            f,
            "{}",
            self.constraints
                .values()
                .map(|v| v.label().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )?;
        write!(f, "]")?;
        write!(f, ">")
    }
}

/// Exception class used when more than one [`ConstraintValueInfo`] for the same
/// [`ConstraintSettingInfo`] is added to a builder.
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:331 ConstraintCollection.DuplicateConstraintException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DuplicateConstraintException {
    duplicate_constraints: IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>>,
    message: String,
}

impl DuplicateConstraintException {
    fn new(
        duplicate_constraints: IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>>,
    ) -> Self {
        let message = Self::format_error(&duplicate_constraints);
        DuplicateConstraintException {
            duplicate_constraints,
            message,
        }
    }

    pub fn duplicate_constraints(
        &self,
    ) -> &IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>> {
        &self.duplicate_constraints
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:346 ConstraintCollection.DuplicateConstraintException.formatError
    pub fn format_error(
        duplicate_constraints: &IndexMap<ConstraintSettingInfo, Vec<ConstraintValueInfo>>,
    ) -> String {
        format!(
            "Duplicate constraint values detected: {}",
            duplicate_constraints
                .iter()
                .map(|(k, v)| Self::describe_single_duplicate_constraint_setting(k, v))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:355 ConstraintCollection.DuplicateConstraintException.describeSingleDuplicateConstraintSetting
    fn describe_single_duplicate_constraint_setting(
        setting: &ConstraintSettingInfo,
        values: &[ConstraintValueInfo],
    ) -> String {
        format!(
            "constraint_setting {} has [{}]",
            setting.label(),
            values
                .iter()
                .map(|v| v.label().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `MethodDescriptor.checkEnabled` for `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API`.
// Java: net/starlark/java/eval/MethodDescriptor.java:409 MethodDescriptor.checkEnabled
pub(crate) fn check_experimental_platforms_api(
    name: &str,
    eval: &Evaluator<'_, '_, '_>,
) -> starlark::Result<()> {
    let cx = ThreadContext::get(eval)?;
    if !cx.semantics.experimental_platforms_api {
        return Err(errorf(format!(
            "function {name}() is experimental and thus unavailable with the current flags. It may be enabled by setting --experimental_platforms_api"
        )));
    }
    Ok(())
}

/// Whether the running thread's semantics enable `--experimental_platforms_api`. A field or
/// method with `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API` is not among the annotated
/// methods of its class otherwise, so `x.f` and `getattr` do not find it and `hasattr` is False
/// (oracle: scripts/w4/scenarios/platforms-api).
// Java: net/starlark/java/eval/CallUtils.java:137 CallUtils.getAnnotatedMethods
// Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
pub(crate) fn experimental_platforms_api(eval: &Evaluator<'_, '_, '_>) -> starlark::Result<bool> {
    Ok(ThreadContext::get(eval)
        .map_err(starlark::Error::new_other)?
        .semantics
        .experimental_platforms_api)
}

/// `get_attr_fallible` of a value whose fields in `gated` are `enableOnlyWithFlag =
/// EXPERIMENTAL_PLATFORMS_API`: `Ok(None)` (missing) for those when the flag is off.
pub(crate) fn gated_get_attr<'v>(
    value: &impl StarlarkValue<'v>,
    gated: &[&str],
    attribute: &str,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Option<Value<'v>>> {
    if gated.contains(&attribute) && !experimental_platforms_api(eval)? {
        return Ok(None);
    }
    Ok(value.get_attr(attribute, eval.heap()))
}

/// Java's message for a missing field of a platform-API value (`Starlark.getattr`: these
/// classes have no unknown-field message of their own). The spelling suggestion is over `fields`
/// without `attribute` itself.
/// PORT: Java suggests from `dir(x)`, which leaves out the gated fields when the flag is off;
/// with no evaluator here the gated fields are candidates for a misspelled name whatever the
/// flag (the name of a gated field itself is never suggested, as in Java without the flag).
// Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
pub(crate) fn no_field_message(type_name: &str, attribute: &str, fields: &[&str]) -> String {
    let candidates: Vec<&str> = fields.iter().copied().filter(|f| *f != attribute).collect();
    format!(
        "'{type_name}' value has no field or method '{attribute}'{}",
        blaze_packages::attribute::string_util::did_you_mean(attribute, &candidates)
    )
}

/// `has_attr_fallible` for [`gated_get_attr`].
pub(crate) fn gated_has_attr<'v>(
    value: &impl StarlarkValue<'v>,
    gated: &[&str],
    attribute: &str,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<bool> {
    if gated.contains(&attribute) && !experimental_platforms_api(eval)? {
        return Ok(false);
    }
    Ok(value.has_attr(attribute, eval.heap()))
}

fn this_collection<'v>(this: Value<'v>) -> &'v ConstraintCollection {
    this.downcast_ref::<ConstraintCollection>()
        .expect("ConstraintCollection method on another value")
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/ConstraintCollectionApi.java:37 ConstraintCollectionApi
#[starlark_module]
fn constraint_collection_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:208 ConstraintCollection.get
    fn get<'v>(
        this: Value<'v>,
        constraint: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check_experimental_platforms_api("get", eval)?;
        let setting = ConstraintCollection::convert_key(constraint)?;
        Ok(match this_collection(this).get(&setting) {
            Some(v) => eval.heap().alloc(v),
            None => Value::new_none(),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:168 ConstraintCollection.has
    fn has<'v>(
        this: Value<'v>,
        constraint: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_experimental_platforms_api("has", eval)?;
        let setting = ConstraintCollection::convert_key(constraint)?;
        Ok(this_collection(this).has(&setting))
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:197 ConstraintCollection.hasConstraintValue
    fn has_constraint_value<'v>(
        this: Value<'v>,
        constraint_value: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_experimental_platforms_api("has_constraint_value", eval)?;
        let Some(v) = constraint_value.downcast_ref::<ConstraintValueInfo>() else {
            return Err(errorf(format!(
                "in call to has_constraint_value(), parameter 'constraint_value' got value of type '{}', want 'ConstraintValueInfo'",
                constraint_value.get_type()
            )));
        };
        Ok(this_collection(this).has_constraint_value(v))
    }
}

#[starlark_value(type = "ConstraintCollection")]
impl<'v> StarlarkValue<'v> for ConstraintCollection {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("ConstraintCollection", constraint_collection_methods);
        Some(RES.methods())
    }

    // `constraint_settings` is `enableOnlyWithFlag = EXPERIMENTAL_PLATFORMS_API` (see
    // `get_attr_fallible`).
    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:224 ConstraintCollection.constraintSettings
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "constraint_settings" => {
                let values: Vec<Value<'v>> = self
                    .constraint_settings()
                    .into_iter()
                    .map(|s| heap.alloc(s))
                    .collect();
                // Java: StarlarkList.immutableCopyOf.
                Some(heap.alloc(AllocList(values)))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "constraint_settings"
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/platform/ConstraintCollectionApi.java:39 ConstraintCollectionApi.constraintSettings
    fn get_attr_fallible(
        &self,
        attribute: Hashed<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        gated_get_attr(self, &["constraint_settings"], attribute.key(), eval)
    }

    fn has_attr_fallible(
        &self,
        attribute: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        gated_has_attr(self, &["constraint_settings"], attribute, eval)
    }

    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(no_field_message(
            "ConstraintCollection",
            attribute,
            &["constraint_settings", "get", "has", "has_constraint_value"],
        ))
    }

    // The methods are listed by `get_methods`.
    // PORT: Java's `dir` leaves out fields disabled by a flag (CallUtils.getAnnotatedMethods);
    // `dir_attr` has no evaluator, so the gated field is listed whatever the flag.
    fn dir_attr(&self) -> Vec<String> {
        vec!["constraint_settings".to_owned()]
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:241 ConstraintCollection.getIndex
    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let setting = Self::convert_key(index)?;
        Ok(match self.get(&setting) {
            Some(v) => heap.alloc(v),
            None => Value::new_none(),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:251 ConstraintCollection.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(self.has(&Self::convert_key(other)?))
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ConstraintCollection>()
            .is_some_and(|o| o == self))
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollection.java:106 ConstraintCollection.isImmutable
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        for (k, v) in self.constraints.iter() {
            k.label().get_canonical_form().hash(hasher);
            v.label().get_canonical_form().hash(hasher);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Oracle: scripts/w4/scenarios/platforms-api steps 3-5 (no --experimental_platforms_api).
    #[test]
    fn gated_field_message_has_no_suggestion_of_itself() {
        assert_eq!(
            no_field_message("PlatformInfo", "label", &["constraints", "label"]),
            "'PlatformInfo' value has no field or method 'label'"
        );
        assert_eq!(
            no_field_message(
                "ConstraintSettingInfo",
                "label",
                &[
                    "default_constraint_value",
                    "has_default_constraint_value",
                    "label"
                ]
            ),
            "'ConstraintSettingInfo' value has no field or method 'label'"
        );
    }
    use blaze_cmdline::Label;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    fn setting(s: &str) -> ConstraintSettingInfo {
        ConstraintSettingInfo::create(label(s))
    }

    fn value(setting: &ConstraintSettingInfo, s: &str) -> ConstraintValueInfo {
        ConstraintValueInfo::create(setting.clone(), label(s))
    }

    fn build(values: &[&ConstraintValueInfo]) -> ConstraintCollection {
        ConstraintCollection::builder()
            .add_constraints(values.iter().map(|v| (*v).clone()))
            .build()
            .unwrap()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollectionTest.java testSetArithmetic
    #[test]
    fn test_set_arithmetic() {
        let setting1 = setting("//foo:s1");
        let value1 = value(&setting1, "//foo:value1");
        let setting2 = setting("//foo:s2");
        let value2 = value(&setting2, "//foo:value2");
        let setting3 = setting("//foo:s3");
        let value3 = value(&setting3, "//foo:value3");

        let collection = build(&[&value1, &value2]);
        assert!(collection.contains_all([&value1]));
        assert!(collection.find_missing([&value1]).is_empty());
        assert!(collection.contains_all([&value2]));
        assert!(collection.contains_all([&value1, &value2]));
        assert!(!collection.contains_all([&value3]));
        assert_eq!(collection.find_missing([&value3]), vec![value3.clone()]);
        assert!(!collection.contains_all([&value1, &value3]));
        assert_eq!(collection.find_missing([&value3]), vec![value3.clone()]);
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollectionTest.java testSetArithmetic_withDefaultValues
    #[test]
    fn test_set_arithmetic_with_default_values() {
        let setting =
            ConstraintSettingInfo::create_with(label("//foo:s"), Some(label("//foo:value1")), None);
        let value1 = value(&setting, "//foo:value1");
        let value2 = value(&setting, "//foo:value2");

        let collection1 = build(&[&value1]);
        assert!(collection1.contains_all([&value1]));
        assert!(collection1.find_missing([&value1]).is_empty());
        assert!(!collection1.contains_all([&value2]));
        assert_eq!(collection1.find_missing([&value2]), vec![value2.clone()]);

        let collection_with_default = build(&[]);
        assert!(collection_with_default.contains_all([&value1]));
        assert!(collection_with_default.find_missing([&value1]).is_empty());
        assert!(!collection_with_default.contains_all([&value2]));
        assert_eq!(
            collection_with_default.find_missing([&value2]),
            vec![value2.clone()]
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollectionTest.java testRepeatedConstraintValue
    #[test]
    fn test_repeated_constraint_value() {
        let setting1 = setting("//foo:s1");
        let value1 = value(&setting1, "//foo:value1");
        let setting2 = setting("//foo:s2");
        let value2 = value(&setting2, "//foo:value2");

        let collection = build(&[&value1, &value2, &value1]);
        assert_eq!(
            collection.constraint_settings(),
            vec![setting1.clone(), setting2.clone()]
        );
        assert_eq!(collection.get(&setting1), Some(value1.clone()));
        assert_eq!(collection.get(&setting2), Some(value2.clone()));
        assert!(collection.contains_all([&value1, &value2]));
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollectionTest.java testDiff
    #[test]
    fn test_diff() {
        let setting1 = setting("//foo:s1");
        let value1 = value(&setting1, "//foo:value1");
        let setting2 = setting("//foo:s2");
        let value2a = value(&setting2, "//foo:value2a");
        let value2b = value(&setting2, "//foo:value2b");

        let collection1 = build(&[&value1, &value2a]);
        let collection2 = build(&[&value1, &value2b]);
        assert_eq!(
            collection1
                .diff(&collection2)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![setting2.clone()]
        );
        assert_eq!(
            collection2
                .diff(&collection1)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![setting2.clone()]
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ConstraintCollectionTest.java testDiff_withParent
    #[test]
    fn test_diff_with_parent() {
        let setting1 = setting("//foo:s1");
        let value1 = value(&setting1, "//foo:value1");
        let setting2 = setting("//foo:s2");
        let value2a = value(&setting2, "//foo:value2a");
        let value2b = value(&setting2, "//foo:value2b");

        let parent1 = build(&[&value2a]);
        let collection1 = ConstraintCollection::builder()
            .parent(Some(parent1.clone()))
            .add_constraints([value1.clone()])
            .build()
            .unwrap();
        let parent2 = build(&[&value2b]);
        let collection2 = ConstraintCollection::builder()
            .parent(Some(parent2))
            .add_constraints([value1.clone()])
            .build()
            .unwrap();

        let only2 = vec![setting2.clone()];
        assert_eq!(
            collection1
                .diff(&collection2)
                .into_iter()
                .collect::<Vec<_>>(),
            only2
        );
        assert_eq!(
            collection2
                .diff(&collection1)
                .into_iter()
                .collect::<Vec<_>>(),
            only2
        );

        let collection3 = ConstraintCollection::builder()
            .parent(Some(parent1))
            .add_constraints([value1.clone()])
            .build()
            .unwrap();
        assert!(collection1.diff(&collection3).is_empty());
        assert!(collection3.diff(&collection1).is_empty());

        let collection4 = build(&[&value1]);
        assert_eq!(
            collection1
                .diff(&collection4)
                .into_iter()
                .collect::<Vec<_>>(),
            only2
        );
        assert_eq!(
            collection4
                .diff(&collection1)
                .into_iter()
                .collect::<Vec<_>>(),
            only2
        );
    }

    #[test]
    fn duplicate_constraint_error_text() {
        let setting1 = setting("//constraint:basic");
        let e = ConstraintCollection::builder()
            .add_constraints([
                value(&setting1, "//constraint:value1"),
                value(&setting1, "//constraint:value2"),
            ])
            .build()
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "Duplicate constraint values detected: constraint_setting //constraint:basic has [//constraint:value1, //constraint:value2]"
        );
    }
}
