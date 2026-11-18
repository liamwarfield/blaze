//! Ports `packages/PackageArgs.java` and `analysis/config/FeatureSet.java` (the latter is
//! needed by `package(features = ...)`).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use indexmap::IndexSet;
use starlark::values::Value;

use crate::package::RuleVisibility;
use crate::types::AttrValue;
use crate::types::ConversionException;
use crate::types::LabelConverter;
use crate::types::License;
use crate::types::Type;

/// Features turned on and off (Java `analysis/config/FeatureSet`, a record of two sorted sets).
// PORT: move to blaze_config (analysis/config) when that crate needs it.
// Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:35 FeatureSet
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FeatureSet {
    pub on: BTreeSet<String>,
    pub off: BTreeSet<String>,
}

impl FeatureSet {
    /// Java `FeatureSet.EMPTY`.
    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:41 FeatureSet.EMPTY
    pub fn empty() -> FeatureSet {
        FeatureSet::default()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:43 FeatureSet.of
    fn of<'a>(
        on: impl IntoIterator<Item = &'a String>,
        off: impl IntoIterator<Item = &'a String>,
    ) -> FeatureSet {
        FeatureSet {
            on: on.into_iter().cloned().collect(),
            off: off.into_iter().cloned().collect(),
        }
    }

    /// Parses a feature set from a list of strings (`-x` turns `x` off).
    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:48 FeatureSet.parse
    pub fn parse(features: &[String]) -> FeatureSet {
        let mut feature_to_state: HashMap<String, bool> = HashMap::new();
        for feature in features {
            if let Some(off) = feature.strip_prefix('-') {
                feature_to_state.insert(off.to_owned(), false);
            } else if feature == "no_layering_check" {
                // TODO(bazel-team): Remove once we do not have BUILD files left that contain
                // 'no_layering_check'.
                feature_to_state.insert("layering_check".to_owned(), false);
            } else {
                // -X always trumps X.
                feature_to_state.entry(feature.clone()).or_insert(true);
            }
        }
        FeatureSet::from_map(&feature_to_state)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:65 FeatureSet.fromMap
    fn from_map(feature_to_state: &HashMap<String, bool>) -> FeatureSet {
        // PORT: the sets are sorted, so the HashMap's order is not observable.
        FeatureSet::of(
            feature_to_state.iter().filter(|(_, v)| **v).map(|(k, _)| k),
            feature_to_state
                .iter()
                .filter(|(_, v)| !**v)
                .map(|(k, _)| k),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:71 FeatureSet.mergeSetIntoMap
    fn merge_set_into_map(
        features: &BTreeSet<String>,
        state: bool,
        feature_to_state: &mut HashMap<String, bool>,
    ) {
        for feature in features {
            feature_to_state.insert(feature.clone(), state);
        }
    }

    /// Merges `coarse` (e.g. package defaults) with `fine` (e.g. rule features); `fine` wins.
    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:83 FeatureSet.merge
    pub fn merge(coarse: &FeatureSet, fine: &FeatureSet) -> FeatureSet {
        let mut feature_to_state = HashMap::new();
        FeatureSet::merge_set_into_map(&coarse.on, true, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&coarse.off, false, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&fine.on, true, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&fine.off, false, &mut feature_to_state);
        FeatureSet::from_map(&feature_to_state)
    }

    /// Merges with the global feature set; globally disabled features are always disabled.
    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:96 FeatureSet.mergeWithGlobalFeatures
    pub fn merge_with_global_features(base: &FeatureSet, global: &FeatureSet) -> FeatureSet {
        let mut feature_to_state = HashMap::new();
        FeatureSet::merge_set_into_map(&global.on, true, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&base.on, true, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&base.off, false, &mut feature_to_state);
        FeatureSet::merge_set_into_map(&global.off, false, &mut feature_to_state);
        FeatureSet::from_map(&feature_to_state)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:35 FeatureSet.on
    pub fn on(&self) -> &BTreeSet<String> {
        &self.on
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:35 FeatureSet.off
    pub fn off(&self) -> &BTreeSet<String> {
        &self.off
    }

    // Java: com/google/devtools/build/lib/analysis/config/FeatureSet.java:105 FeatureSet.toStringList
    pub fn to_string_list(&self) -> Vec<String> {
        self.on
            .iter()
            .cloned()
            .chain(self.off.iter().map(|s| format!("-{s}")))
            .collect()
    }
}

/// Arguments of `package()` / `repo()` that affect a package's targets (Java `PackageArgs`).
/// `None` fields are unset (Java null).
// Java: com/google/devtools/build/lib/packages/PackageArgs.java:37 PackageArgs
#[derive(Debug, Clone, PartialEq)]
pub struct PackageArgs {
    default_visibility: Option<RuleVisibility>,
    default_test_only: Option<bool>,
    default_deprecation: Option<String>,
    features: FeatureSet,
    license: Option<License>,
    default_compatible_with: Option<IndexSet<Label>>,
    default_restricted_to: Option<IndexSet<Label>>,
    default_package_metadata: Option<Vec<Label>>,
    transitive_visibility: Option<Label>,
    default_hdrs_check: Option<String>,
}

impl PackageArgs {
    /// Java `PackageArgs.EMPTY`.
    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:38 PackageArgs.EMPTY
    pub fn empty() -> PackageArgs {
        PackageArgs::builder().build()
    }

    /// Java `PackageArgs.DEFAULT`.
    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:40 PackageArgs.DEFAULT
    pub fn default_args() -> PackageArgs {
        let mut b = PackageArgs::builder();
        b.set_default_visibility(RuleVisibility::Private)
            .set_default_test_only(false)
            .set_features(FeatureSet::empty())
            .set_license(License::no_license())
            .set_default_compatible_with(Vec::new())
            .set_default_restricted_to(Vec::new())
            .set_default_package_metadata(Vec::new());
        b.build()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:53 PackageArgs.defaultVisibility
    pub fn default_visibility(&self) -> Option<&RuleVisibility> {
        self.default_visibility.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:57 PackageArgs.defaultTestOnly
    pub fn default_test_only(&self) -> Option<bool> {
        self.default_test_only
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:61 PackageArgs.defaultDeprecation
    pub fn default_deprecation(&self) -> Option<&str> {
        self.default_deprecation.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:69 PackageArgs.features
    pub fn features(&self) -> &FeatureSet {
        &self.features
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:73 PackageArgs.license
    pub fn license(&self) -> Option<&License> {
        self.license.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:77 PackageArgs.defaultCompatibleWith
    pub fn default_compatible_with(&self) -> Option<&IndexSet<Label>> {
        self.default_compatible_with.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:81 PackageArgs.defaultRestrictedTo
    pub fn default_restricted_to(&self) -> Option<&IndexSet<Label>> {
        self.default_restricted_to.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:85 PackageArgs.defaultPackageMetadata
    pub fn default_package_metadata(&self) -> Option<&[Label]> {
        self.default_package_metadata.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:89 PackageArgs.transitiveVisibility
    pub fn transitive_visibility(&self) -> Option<&Label> {
        self.transitive_visibility.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:98 PackageArgs.getDefaultHdrsCheck
    pub fn get_default_hdrs_check(&self) -> &str {
        self.default_hdrs_check.as_deref().unwrap_or("strict")
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:103 PackageArgs.isDefaultHdrsCheckSet
    pub fn is_default_hdrs_check_set(&self) -> bool {
        self.default_hdrs_check.is_some()
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:107 PackageArgs.builder
    pub fn builder() -> PackageArgsBuilder {
        PackageArgsBuilder {
            args: PackageArgs {
                default_visibility: None,
                default_test_only: None,
                default_deprecation: None,
                features: FeatureSet::empty(),
                license: None,
                default_compatible_with: None,
                default_restricted_to: None,
                default_package_metadata: None,
                transitive_visibility: None,
                default_hdrs_check: None,
            },
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:111 PackageArgs.toBuilder
    pub fn to_builder(&self) -> PackageArgsBuilder {
        PackageArgsBuilder { args: self.clone() }
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:152 PackageArgs.throwIfHasDupes
    fn throw_if_has_dupes(labels: Vec<Label>, what: &str) -> Result<Vec<Label>, EvalException> {
        // Java: com/google/devtools/build/lib/collect/CollectionUtils.java:37 CollectionUtils.duplicatedElementsOf
        let mut element_set: HashSet<&Label> = HashSet::new();
        let mut dupes: BTreeSet<&Label> = BTreeSet::new();
        if labels.len() >= 2 {
            for el in &labels {
                if !element_set.insert(el) {
                    dupes.insert(el);
                }
            }
        }
        if !dupes.is_empty() {
            let joined: Vec<String> = dupes.iter().map(|l| l.to_string()).collect();
            return Err(EvalException::new(format!(
                "duplicate label(s) in {}: {}",
                what,
                joined.join(", ")
            )));
        }
        Ok(labels)
    }

    /// Processes the Starlark parameter `name` of a `package()` / `repo()` call into `builder`.
    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:164 PackageArgs.processParam
    pub fn process_param<'v>(
        name: &str,
        raw_value: Value<'v>,
        what: &str,
        label_converter: &LabelConverter,
        builder: &mut PackageArgsBuilder,
    ) -> Result<(), EvalException> {
        let convert = |type_: Type| -> Result<AttrValue, EvalException> {
            type_
                .convert(
                    raw_value,
                    Some(&what as &dyn fmt::Display),
                    Some(label_converter),
                )
                .map_err(conversion_to_eval)
        };
        match name {
            "default_visibility" => {
                let labels = expect_label_list(convert(Type::LabelList)?);
                builder.set_default_visibility(RuleVisibility::parse(&labels)?);
            }
            "default_testonly" => {
                builder.set_default_test_only(expect_bool(convert(Type::Boolean)?));
            }
            "default_deprecation" => {
                builder.set_default_deprecation(expect_string(convert(Type::String)?));
            }
            "features" => {
                let features = expect_string_list(convert(Type::StringList)?);
                builder.merge_features(&FeatureSet::parse(&features));
            }
            "licenses" => {
                builder.set_license(expect_license(convert(Type::License)?));
            }
            "default_compatible_with" => {
                let labels = expect_label_list(convert(Type::LabelList)?);
                builder.set_default_compatible_with(Self::throw_if_has_dupes(labels, name)?);
            }
            "default_restricted_to" => {
                let labels = expect_label_list(convert(Type::LabelList)?);
                builder.set_default_restricted_to(Self::throw_if_has_dupes(labels, name)?);
            }
            "default_applicable_licenses" | "default_package_metadata" => {
                if builder.default_package_metadata().is_some() {
                    return Err(EvalException::new(
                        "Can not set both default_package_metadata and \
                         default_applicable_licenses. Move all declarations to \
                         default_package_metadata.",
                    ));
                }
                let labels = expect_label_list(convert(Type::LabelList)?);
                builder.set_default_package_metadata(Self::throw_if_has_dupes(labels, name)?);
            }
            "default_hdrs_check" => {
                builder.set_default_hdrs_check(expect_string(convert(Type::String)?));
            }
            "transitive_visibility" => {
                let transitive_visibility = match convert(Type::Label)? {
                    AttrValue::Label(l) => l,
                    other => panic!("ClassCastException: expected Label, got {other:?}"),
                };
                builder.set_transitive_visibility(transitive_visibility);
            }
            _ => {
                return Err(EvalException::new(format!(
                    "unexpected keyword argument: {name}"
                )));
            }
        }
        Ok(())
    }

    /// `other` merged into `self`; `other`'s set fields take precedence.
    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:209 PackageArgs.mergeWith
    pub fn merge_with(&self, other: &PackageArgs) -> PackageArgs {
        let mut builder = self.to_builder();
        if let Some(x) = &other.default_visibility {
            builder.set_default_visibility(x.clone());
        }
        if let Some(x) = other.default_test_only {
            builder.set_default_test_only(x);
        }
        if let Some(x) = &other.default_deprecation {
            builder.set_default_deprecation(x.clone());
        }
        if other.features != FeatureSet::empty() {
            builder.merge_features(&other.features);
        }
        if let Some(x) = &other.license {
            builder.set_license(x.clone());
        }
        if let Some(x) = &other.default_compatible_with {
            builder.set_default_compatible_with(x.iter().cloned().collect());
        }
        if let Some(x) = &other.default_restricted_to {
            builder.set_default_restricted_to(x.iter().cloned().collect());
        }
        if let Some(x) = &other.default_package_metadata {
            builder.set_default_package_metadata(x.clone());
        }
        if let Some(x) = &other.transitive_visibility {
            builder.set_transitive_visibility(x.clone());
        }
        if let Some(x) = &other.default_hdrs_check {
            builder.set_default_hdrs_check(x.clone());
        }
        builder.build()
    }
}

/// Java `PackageArgs.Builder`.
// Java: com/google/devtools/build/lib/packages/PackageArgs.java:115 PackageArgs.Builder
#[derive(Debug, Clone)]
pub struct PackageArgsBuilder {
    args: PackageArgs,
}

impl PackageArgsBuilder {
    pub fn set_default_visibility(&mut self, x: RuleVisibility) -> &mut Self {
        self.args.default_visibility = Some(x);
        self
    }

    pub fn set_default_test_only(&mut self, x: bool) -> &mut Self {
        self.args.default_test_only = Some(x);
        self
    }

    pub fn set_default_deprecation(&mut self, x: String) -> &mut Self {
        self.args.default_deprecation = Some(x);
        self
    }

    pub fn features(&self) -> &FeatureSet {
        &self.args.features
    }

    pub fn set_features(&mut self, x: FeatureSet) -> &mut Self {
        self.args.features = x;
        self
    }

    // Java: com/google/devtools/build/lib/packages/PackageArgs.java:127 PackageArgs.Builder.mergeFeatures
    pub fn merge_features(&mut self, x: &FeatureSet) -> &mut Self {
        let merged = FeatureSet::merge(&self.args.features, x);
        self.set_features(merged)
    }

    pub fn set_license(&mut self, x: License) -> &mut Self {
        self.args.license = Some(x);
        self
    }

    pub fn set_default_compatible_with(&mut self, x: Vec<Label>) -> &mut Self {
        self.args.default_compatible_with = Some(x.into_iter().collect());
        self
    }

    pub fn set_default_restricted_to(&mut self, x: Vec<Label>) -> &mut Self {
        self.args.default_restricted_to = Some(x.into_iter().collect());
        self
    }

    pub fn default_package_metadata(&self) -> Option<&[Label]> {
        self.args.default_package_metadata.as_deref()
    }

    pub fn set_default_package_metadata(&mut self, x: Vec<Label>) -> &mut Self {
        self.args.default_package_metadata = Some(x);
        self
    }

    pub fn set_transitive_visibility(&mut self, x: Label) -> &mut Self {
        self.args.transitive_visibility = Some(x);
        self
    }

    pub fn set_default_hdrs_check(&mut self, x: String) -> &mut Self {
        self.args.default_hdrs_check = Some(x);
        self
    }

    pub fn build(&self) -> PackageArgs {
        self.args.clone()
    }
}

/// Java: `ConversionException extends EvalException`; the message is kept.
fn conversion_to_eval(e: ConversionException) -> EvalException {
    EvalException::new(e.get_message())
}

// PORT: Java casts the converted `Object`; a mismatch is a ClassCastException (a crash).
fn expect_label_list(v: AttrValue) -> Vec<Label> {
    match v {
        AttrValue::LabelList(l) => l,
        other => panic!("ClassCastException: expected List<Label>, got {other:?}"),
    }
}

fn expect_string_list(v: AttrValue) -> Vec<String> {
    match v {
        AttrValue::StringList(l) => l,
        other => panic!("ClassCastException: expected List<String>, got {other:?}"),
    }
}

fn expect_string(v: AttrValue) -> String {
    match v {
        AttrValue::String(s) => s,
        other => panic!("ClassCastException: expected String, got {other:?}"),
    }
}

fn expect_bool(v: AttrValue) -> bool {
    match v {
        AttrValue::Bool(b) => b,
        other => panic!("ClassCastException: expected Boolean, got {other:?}"),
    }
}

fn expect_license(v: AttrValue) -> License {
    match v {
        AttrValue::License(l) => l,
        other => panic!("ClassCastException: expected License, got {other:?}"),
    }
}
