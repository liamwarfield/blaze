//! Starlark values over the `blaze_cc_features` engine: `CcToolchainVariables` (type
//! `Variables`), `FeatureConfigurationForStarlark` (type `FeatureConfiguration`, methods
//! `is_enabled` / `is_requested`), `CcToolchainFeatures` (methods `configure_features`,
//! `default_features_and_action_configs`), and the conversion of a `CcToolchainConfigInfo`
//! provider instance into the Starlark-free tree `blaze_cc_features::starlark_info`.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_cc_features::cc_toolchain_features::CcToolchainFeatures;
use blaze_cc_features::cc_toolchain_variables::CcToolchainVariablesRef;
use blaze_cc_features::feature_configuration_for_starlark::FeatureConfigurationForStarlark;
use blaze_cc_features::starlark_info::{
    StarlarkInfo as CcStarlarkInfo, StarlarkInfoValue as CcStarlarkInfoValue,
};
use blaze_starlark::structs::{DeeplyImmutable, StarlarkInfo, provider_info};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::tuple::TupleRef;
use starlark::values::{Demand, NoSerialize, StarlarkValue, Value, starlark_value};

use super::cc_artifact::to_cc_artifact;
use super::util::{check_private_starlarkification_allowlist, errorf, sequence_cast_strings};

// -------------------------------------------------------------------------------------------
// CcToolchainVariables

/// The Starlark value of a `CcToolchainVariables`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:65 CcToolchainVariables
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct CcToolchainVariablesValue(#[allocative(skip)] pub CcToolchainVariablesRef);

starlark_simple_value!(CcToolchainVariablesValue);

impl fmt::Debug for CcToolchainVariablesValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CcToolchainVariablesValue({:?})", self.0)
    }
}

impl fmt::Display for CcToolchainVariablesValue {
    // Java: StarlarkValue.repr default (`<unknown object <class name>>`).
    // PORT: Java prints the runtime subclass (`CcToolchainVariables$MapVariables`, ...), which
    // the Rust enum does not expose; the base class name is printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<unknown object com.google.devtools.build.lib.rules.cpp.CcToolchainVariables>")
    }
}

#[starlark_value(type = "Variables")]
impl<'v> StarlarkValue<'v> for CcToolchainVariablesValue {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:953 CcToolchainVariables.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// The variables behind a `Variables` value, or Java's parameter type error.
pub fn variables_param(
    x: Value,
    function: &str,
    param: &str,
) -> starlark::Result<CcToolchainVariablesRef> {
    x.downcast_ref::<CcToolchainVariablesValue>()
        .map(|v| v.0.clone())
        .ok_or_else(|| {
            errorf(format!(
                "in call to {function}(), parameter '{param}' got value of type '{}', want \
                 'Variables'",
                x.get_type()
            ))
        })
}

// -------------------------------------------------------------------------------------------
// FeatureConfigurationForStarlark

/// The Starlark value of a `FeatureConfigurationForStarlark`.
// Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:38 FeatureConfigurationForStarlark
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FeatureConfigurationValue(#[allocative(skip)] pub Arc<FeatureConfigurationForStarlark>);

starlark_simple_value!(FeatureConfigurationValue);

impl fmt::Debug for FeatureConfigurationValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.debug_print())
    }
}

impl fmt::Display for FeatureConfigurationValue {
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:55 FeatureConfigurationForStarlark.str
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.str())
    }
}

#[starlark_value(type = "FeatureConfiguration")]
impl<'v> StarlarkValue<'v> for FeatureConfigurationValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("FeatureConfiguration", feature_configuration_methods);
        Some(RES.methods())
    }

    // Java: StarlarkValue.repr default.
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(
            "<unknown object com.google.devtools.build.lib.rules.cpp.FeatureConfigurationForStarlark>",
        );
    }
}

#[starlark_module]
fn feature_configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:76 FeatureConfigurationForStarlark.isRequested
    fn is_requested<'v>(
        this: &FeatureConfigurationValue,
        #[starlark(require = pos)] feature: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.0.is_requested(feature))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:87 FeatureConfigurationForStarlark.isEnabled
    fn is_enabled<'v>(
        this: &FeatureConfigurationValue,
        #[starlark(require = pos)] feature: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.0.is_enabled(feature))
    }
}

/// The feature configuration behind a `FeatureConfiguration` value, or Java's parameter type
/// error.
pub fn feature_configuration_param(
    x: Value,
    function: &str,
    param: &str,
) -> starlark::Result<Arc<FeatureConfigurationForStarlark>> {
    x.downcast_ref::<FeatureConfigurationValue>()
        .map(|v| v.0.clone())
        .ok_or_else(|| {
            errorf(format!(
                "in call to {function}(), parameter '{param}' got value of type '{}', want \
                 'FeatureConfiguration'",
                x.get_type()
            ))
        })
}

// -------------------------------------------------------------------------------------------
// CcToolchainFeatures

/// The Starlark value of `CcToolchainFeatures` (Java's class is itself a `StarlarkValue`
/// without `@StarlarkBuiltin`, so its type is the class name).
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:80 CcToolchainFeatures
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct CcToolchainFeaturesValue(#[allocative(skip)] pub Arc<CcToolchainFeatures>);

starlark_simple_value!(CcToolchainFeaturesValue);

impl fmt::Debug for CcToolchainFeaturesValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CcToolchainFeatures")
    }
}

impl fmt::Display for CcToolchainFeaturesValue {
    // Java: StarlarkValue.repr default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<unknown object com.google.devtools.build.lib.rules.cpp.CcToolchainFeatures>")
    }
}

#[starlark_value(type = "CcToolchainFeatures")]
impl<'v> StarlarkValue<'v> for CcToolchainFeaturesValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("CcToolchainFeatures", cc_toolchain_features_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn cc_toolchain_features_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1234 CcToolchainFeatures.configureFeatures
    fn configure_features<'v>(
        this: &CcToolchainFeaturesValue,
        requested_features: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<FeatureConfigurationValue> {
        // Java declares `StarlarkList<?> requestedFeatures`.
        if ListRef::from_value(requested_features).is_none() {
            return Err(errorf(format!(
                "in call to configure_features(), parameter 'requested_features' got value of \
                 type '{}', want 'list'",
                requested_features.get_type()
            )));
        }
        check_private_starlarkification_allowlist(eval)?;
        let requested =
            sequence_cast_strings(requested_features, "requested_features", eval.heap())?;
        // Java: catch (CollidingProvidesException ex) { throw new EvalException(ex); }
        let fc = this
            .0
            .configure_features(&requested)
            .map_err(|e| errorf(e.message))?;
        Ok(FeatureConfigurationValue(Arc::new(fc)))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1305 CcToolchainFeatures.getDefaultFeaturesAndActionConfigsForStarlark
    fn default_features_and_action_configs<'v>(
        this: &CcToolchainFeaturesValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check_private_starlarkification_allowlist(eval)?;
        let names: Vec<Value<'v>> = this
            .0
            .get_default_features_and_action_configs()
            .iter()
            .map(|s| eval.heap().alloc_str(s).to_value())
            .collect();
        Ok(eval.heap().alloc(AllocList(names)))
    }
}

/// The features behind a `CcToolchainFeatures` value (Java `value.getValue(...,
/// CcToolchainFeatures.class)`).
pub fn cc_toolchain_features_of(x: Value) -> Option<Arc<CcToolchainFeatures>> {
    x.downcast_ref::<CcToolchainFeaturesValue>()
        .map(|v| v.0.clone())
}

// -------------------------------------------------------------------------------------------
// StarlarkInfo -> blaze_cc_features tree

/// Converts a struct or provider instance to the tree `CcToolchainFeaturesLib` reads.
// PORT: Java reads the StarlarkInfo objects directly; the engine crate has no Starlark, so the
// provider instance is converted. Creation locations of Blaze struct values are not recorded;
// they only appear in error messages and are rendered as "<unknown>".
pub fn to_cc_starlark_info(info: &StarlarkInfo) -> CcStarlarkInfo {
    let printable_name = provider_info(info.provider())
        .map(|p| p.printable_name)
        .unwrap_or_else(|| "struct".to_owned());
    let mut out = CcStarlarkInfo::new(&printable_name, "<unknown>");
    for (name, value) in info.fields() {
        out.fields
            .insert(name.to_owned(), to_cc_starlark_info_value(value));
    }
    out
}

/// Converts a Starlark value to the tree `CcToolchainFeaturesLib` reads.
pub fn to_cc_starlark_info_value(v: Value) -> CcStarlarkInfoValue {
    to_value(v)
}

fn to_value(v: Value) -> CcStarlarkInfoValue {
    if v.is_none() {
        return CcStarlarkInfoValue::None;
    }
    if let Some(b) = v.unpack_bool() {
        return CcStarlarkInfoValue::Bool(b);
    }
    if let Some(i) = v.unpack_i32() {
        return CcStarlarkInfoValue::Int(i as i64);
    }
    if let Some(s) = v.unpack_str() {
        return CcStarlarkInfoValue::String(s.to_owned());
    }
    if let Some(l) = ListRef::from_value(v) {
        return CcStarlarkInfoValue::List(l.content().iter().map(|x| to_value(*x)).collect());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return CcStarlarkInfoValue::List(t.content().iter().map(|x| to_value(*x)).collect());
    }
    if let Some(info) = StarlarkInfo::from_value(v) {
        return CcStarlarkInfoValue::Info(Arc::new(to_cc_starlark_info(info)));
    }
    if let Some(a) = artifact_from_value(v) {
        return CcStarlarkInfoValue::Artifact(to_cc_artifact(&a));
    }
    CcStarlarkInfoValue::Other(v.get_type().to_owned())
}
