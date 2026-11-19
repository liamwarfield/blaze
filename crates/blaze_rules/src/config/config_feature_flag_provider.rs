//! Port of `rules/config/ConfigFeatureFlagProvider.java`: provider for exporting value and valid
//! value predicate of feature flags to consuming targets (Starlark `FeatureFlagInfo`).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::{InfoProviderKey, downcast_info};
use blaze_packages::starlark_defs::builtin_providers::{
    BuiltinConstructor, BuiltinProviderData, BuiltinProviderId,
};
use blaze_packages::starlark_defs::config_common::{
    ConfigFeatureFlagProviderConstructor, FEATURE_FLAG_INFO,
};
use blaze_starlark::structs::StructureMarker;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, ValueLike, starlark_value,
};

/// Java `Predicate<String>` validity predicate.
pub type ValidityPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Provider for exporting value and valid value predicate of feature flags to consuming
/// targets.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:38 ConfigFeatureFlagProvider
#[derive(Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ConfigFeatureFlagProvider {
    #[allocative(skip)]
    value: Option<String>,
    #[allocative(skip)]
    potential_error: Option<String>,
    #[allocative(skip)]
    validity_predicate: ValidityPredicate,
}

starlark_simple_value!(ConfigFeatureFlagProvider);

impl fmt::Debug for ConfigFeatureFlagProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigFeatureFlagProvider")
            .field("value", &self.value)
            .field("potential_error", &self.potential_error)
            .finish()
    }
}

impl ConfigFeatureFlagProvider {
    /// Creates a new ConfigFeatureFlagProvider with the given value and valid value predicate.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:70 ConfigFeatureFlagProvider.create
    pub fn create(
        value: Option<String>,
        potential_error: Option<String>,
        is_valid_value: ValidityPredicate,
    ) -> ConfigFeatureFlagProvider {
        ConfigFeatureFlagProvider {
            value,
            potential_error,
            validity_predicate: is_valid_value,
        }
    }

    /// Retrieves and casts the provider from the given target.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:110 ConfigFeatureFlagProvider.fromTarget
    pub fn from_target(target: &ConfiguredTarget) -> Option<OwnedFrozenValue> {
        target.get(&FEATURE_FLAG_INFO.key())
    }

    /// Java `((ConfigFeatureFlagProvider) info)` for a frozen info.
    pub fn of(info: &OwnedFrozenValue) -> &ConfigFeatureFlagProvider {
        downcast_info::<ConfigFeatureFlagProvider>(info)
            .expect("ClassCastException: not a ConfigFeatureFlagProvider")
    }

    /// Gets the current value of the flag in the flag's current configuration (`None` when
    /// `get_error()` is non-empty).
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:120 ConfigFeatureFlagProvider.getFlagValue
    pub fn get_flag_value(&self) -> Option<&str> {
        if self
            .potential_error
            .as_deref()
            .is_some_and(|e| !e.is_empty())
        {
            return None;
        }
        self.value.as_deref()
    }

    /// Gets the current value of the flag in the flag's current configuration.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:130 ConfigFeatureFlagProvider.getError
    pub fn get_error(&self) -> Option<&str> {
        self.potential_error.as_deref()
    }

    /// Returns whether this value is valid for this flag.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:137 ConfigFeatureFlagProvider.isValidValue
    pub fn is_valid_value(&self, value: &str) -> bool {
        (self.validity_predicate)(value)
    }
}

impl fmt::Display for ConfigFeatureFlagProvider {
    // Java: StarlarkValue.repr default of a NativeInfo (StructImpl.repr over the struct fields).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct(")?;
        match self.get_error() {
            Some(e) => write!(f, "error = {:?}, ", e)?,
            None => write!(f, "error = None, ")?,
        }
        match self.get_flag_value() {
            Some(v) => write!(f, "value = {:?})", v),
            None => write!(f, "value = None)"),
        }
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/config/ConfigFeatureFlagProviderApi.java:25 ConfigFeatureFlagProviderApi
#[starlark_value(type = "FeatureFlagInfo")]
impl<'v> StarlarkValue<'v> for ConfigFeatureFlagProvider {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("FeatureFlagInfo", feature_flag_info_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "value" => Some(match self.get_flag_value() {
                Some(v) => heap.alloc(v),
                None => Value::new_none(),
            }),
            "error" => Some(match self.get_error() {
                Some(e) => heap.alloc(e),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "value" | "error")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["error".to_owned(), "value".to_owned()]
    }

    // ConfigFeatureFlagProvider instances should all be unique, so we override the default
    // equals and hashCode from Info to ensure that.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:144 ConfigFeatureFlagProvider.equals
    // (identity; starlark-rust compares pointers before calling `equals`).
    fn equals(&self, _other: Value<'v>) -> starlark::Result<bool> {
        Ok(false)
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(FEATURE_FLAG_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

#[starlark_module]
fn feature_flag_info_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:137 ConfigFeatureFlagProvider.isValidValue
    fn is_valid_value<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] value: &str,
    ) -> starlark::Result<bool> {
        let me = this
            .downcast_ref::<ConfigFeatureFlagProvider>()
            .expect("FeatureFlagInfo");
        Ok(me.is_valid_value(value))
    }
}

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `Constructor.selfcall`: `@Param(name = "value", named = true)` (positional too), a
/// `String`.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:96 ConfigFeatureFlagProvider.Constructor.selfcall
fn feature_flag_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let positions: Vec<Value<'v>> = args.positions(eval.heap())?.collect();
    let names = args.names_map()?;
    let mut value: Option<Value<'v>> = None;
    if positions.len() > 1 {
        return Err(errorf(format!(
            "FeatureFlagInfo() accepts no more than 1 positional argument but got {}",
            positions.len()
        )));
    }
    if let Some(v) = positions.first() {
        value = Some(*v);
    }
    for (k, v) in names.iter() {
        if k.as_str() != "value" {
            return Err(errorf(format!(
                "FeatureFlagInfo() got unexpected keyword argument: {}",
                k.as_str()
            )));
        }
        if value.is_some() {
            return Err(errorf(
                "FeatureFlagInfo() got multiple values for argument 'value'",
            ));
        }
        value = Some(*v);
    }
    let Some(value) = value else {
        return Err(errorf(
            "FeatureFlagInfo() missing 1 required positional argument: value",
        ));
    };
    let Some(value) = value.unpack_str() else {
        return Err(errorf(format!(
            "in call to FeatureFlagInfo(), parameter 'value' got value of type '{}', want 'string'",
            value.get_type()
        )));
    };
    Ok(eval.heap().alloc(ConfigFeatureFlagProvider::create(
        Some(value.to_owned()),
        None,
        Arc::new(|_| true),
    )))
}

/// Constructor and identifier for ConfigFeatureFlagProvider. This is the value of
/// `config_common.FeatureFlagInfo`.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagProvider.java:47 ConfigFeatureFlagProvider.STARLARK_CONSTRUCTOR
pub const fn starlark_constructor() -> ConfigFeatureFlagProviderConstructor {
    ConfigFeatureFlagProviderConstructor {
        provider: BuiltinProviderData {
            id: FEATURE_FLAG_INFO,
            constructor: BuiltinConstructor::Native(feature_flag_info_constructor),
        },
    }
}
