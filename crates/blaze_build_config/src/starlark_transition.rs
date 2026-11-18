//! Starlark-defined configuration transitions: `StarlarkDefinedConfigTransition` (Java
//! `analysis/config/StarlarkDefinedConfigTransition.java`), the `exec_transition()` builtin
//! (`rules/config/ConfigGlobalLibrary.execTransition`), the Java objects that flow between
//! option values and Starlark, and the loading of the bundled
//! `@_builtins//:common/builtin_exec_platforms.bzl` (`StarlarkExecTransitionLoader`).
//!
//! PORT: move the Starlark value and `transition()`/`exec_transition()` to the crate that owns
//! `ConfigGlobalLibrary` (blaze_config / the rule-definition builtins) and the .bzl loading to the
//! shared `@_builtins` loader (blaze_sky_loading STARLARK_BUILTINS + BZL_LOAD) once they can load
//! `@_builtins` files; this module evaluates the two builtins files the exec transition needs
//! itself, from copies bundled in this crate (Bazel bundles them in its server jar).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::label_constants::{
    COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER, COMMAND_LINE_OPTION_PREFIX,
};
use blaze_cmdline::{Label, LabelPackageContext, RepositoryMapping};
use blaze_events::{Event, EventHandler};
use blaze_options::Value as OptionValue;
use blaze_options::definition::OptionDefinitionRef;
use blaze_starlark::label::LabelValue;
use blaze_starlark::thread::{Phase, StarlarkEvents, ThreadContext};
use blaze_starlark::{EvalError, FileKind, Location, StarlarkSemantics};
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::{FrozenModule, GlobalsBuilder};
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::{AllocDict, DictRef};
use starlark::values::list::{AllocList, ListRef};
use starlark::values::none::NoneType;
use starlark::values::tuple::TupleRef;
use starlark::values::{
    Freeze, FreezeResult, Freezer, FrozenValue, NoSerialize, OwnedFrozenValue, StarlarkValue,
    Trace, Value, ValueLike, starlark_value,
};

use crate::options_diff::OptionInfo;

/// `PATCH_TRANSITION_KEY` (`ConfigurationTransition.PATCH_TRANSITION_KEY`).
pub const PATCH_TRANSITION_KEY: &str = "";

// -------------------------------------------------------------------------------------------
// Java objects.

/// A Java `Object` passed between option values and Starlark: a build setting value read from
/// `BuildOptions` (`findNativeOptionValue`) or a value a transition function returned.
#[derive(Clone, Debug)]
pub enum JavaObject {
    /// Java `null` / `Starlark.NONE`.
    Null,
    Bool(bool),
    /// `Integer` from an option, `StarlarkInt` from Starlark.
    Int(i64),
    Float(f64),
    Str(String),
    Label(Label),
    /// A Java `List` (`ImmutableList` from an option, `StarlarkList` from Starlark).
    List(Vec<JavaObject>),
    /// A Starlark `Tuple` (also a `java.util.List`, but never equal to a list).
    Tuple(Vec<JavaObject>),
    /// A Java `Map` (`Dict` from Starlark).
    Map(Vec<(JavaObject, JavaObject)>),
    /// An option value whose Java class implements `StarlarkValue` (enums such as
    /// `CompilationMode` and `TriState`): Starlark sees the object itself.
    StarlarkValue(OptionValue),
    /// Any other option value (not readable by Starlark without conversion).
    Other(OptionValue),
}

impl PartialEq for JavaObject {
    /// Java `Objects.equals`.
    fn eq(&self, other: &Self) -> bool {
        use JavaObject::*;
        match (self, other) {
            (Null, Null) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Label(a), Label(b)) => a == b,
            (List(a), List(b)) => a == b,
            (Tuple(a), Tuple(b)) => a == b,
            (Map(a), Map(b)) => a.len() == b.len() && a.iter().all(|e| b.contains(e)),
            (StarlarkValue(a), StarlarkValue(b)) => a == b,
            (Other(a), Other(b)) => a == b,
            _ => false,
        }
    }
}

impl JavaObject {
    /// Java `String.valueOf(object)`.
    pub fn java_to_string(&self) -> String {
        match self {
            JavaObject::Null => "null".to_string(),
            JavaObject::Bool(b) => b.to_string(),
            JavaObject::Int(i) => i.to_string(),
            JavaObject::Float(f) => blaze_options::value::java_double_to_string(*f),
            JavaObject::Str(s) => s.clone(),
            JavaObject::Label(l) => l.to_string(),
            JavaObject::List(l) => {
                blaze_options::value::java_list_to_string(l.iter().map(|e| e.java_to_string()))
            }
            // Java: net/starlark/java/eval/Tuple.java:139 Tuple.toString (the Starlark repr).
            // PORT: only reached for a tuple nested in a value given to a native list option.
            JavaObject::Tuple(_) => {
                blaze_util::unported!("Tuple.toString", "tuple as an element of an option value")
            }
            JavaObject::Map(m) => {
                let entries: Vec<String> = m
                    .iter()
                    .map(|(k, v)| format!("{}={}", k.java_to_string(), v.java_to_string()))
                    .collect();
                format!("{{{}}}", entries.join(", "))
            }
            JavaObject::StarlarkValue(v) | JavaObject::Other(v) => v.java_to_string(),
        }
    }

    /// Java `Starlark.type(x)` for the error messages that name a value's type.
    pub fn starlark_type(&self) -> &'static str {
        match self {
            JavaObject::Null => "NoneType",
            JavaObject::Bool(_) => "bool",
            JavaObject::Int(_) => "int",
            JavaObject::Float(_) => "float",
            JavaObject::Str(_) => "string",
            JavaObject::Label(_) => "Label",
            JavaObject::List(_) => "list",
            JavaObject::Tuple(_) => "tuple",
            JavaObject::Map(_) => "dict",
            JavaObject::StarlarkValue(_) | JavaObject::Other(_) => "unknown",
        }
    }

    /// Classifies an option value by its Java class.
    pub fn from_option_value(value: Option<&OptionValue>) -> JavaObject {
        let Some(v) = value else {
            return JavaObject::Null;
        };
        if let Some(s) = v.downcast_ref::<String>() {
            return JavaObject::Str(s.clone());
        }
        if let Some(b) = v.downcast_ref::<bool>() {
            return JavaObject::Bool(*b);
        }
        if let Some(i) = v.downcast_ref::<i32>() {
            return JavaObject::Int(*i as i64);
        }
        if let Some(i) = v.downcast_ref::<i64>() {
            return JavaObject::Int(*i);
        }
        if let Some(f) = v.downcast_ref::<f64>() {
            return JavaObject::Float(*f);
        }
        if let Some(l) = v.downcast_ref::<blaze_options::Foreign<Label>>() {
            return JavaObject::Label(l.0.clone());
        }
        if let Some(list) = v.as_list() {
            return JavaObject::List(
                list.iter()
                    .map(|e| JavaObject::from_option_value(Some(e)))
                    .collect(),
            );
        }
        if is_java_starlark_value(v) {
            return JavaObject::StarlarkValue(v.clone());
        }
        JavaObject::Other(v.clone())
    }

    /// The option value for this object when it is already of the option's Java type
    /// (`def.setValue(options, object)`).
    pub fn to_option_value(&self) -> Option<OptionValue> {
        match self {
            JavaObject::Null => None,
            JavaObject::Bool(b) => Some(OptionValue::new(*b)),
            // An Integer (converted from StarlarkInt) only reaches an `int` field.
            JavaObject::Int(i) => Some(OptionValue::new(*i as i32)),
            JavaObject::Float(f) => Some(OptionValue::new(*f)),
            JavaObject::Str(s) => Some(OptionValue::new(s.clone())),
            JavaObject::Label(l) => Some(OptionValue::new(blaze_options::Foreign(l.clone()))),
            JavaObject::List(l) | JavaObject::Tuple(l) => Some(OptionValue::list(
                l.iter().filter_map(|e| e.to_option_value()).collect(),
            )),
            JavaObject::Map(_) => {
                blaze_util::unported!("FunctionTransitionUtil.applyTransition", "Map option value")
            }
            JavaObject::StarlarkValue(v) | JavaObject::Other(v) => Some(v.clone()),
        }
    }
}

/// Whether the Java class of an option value implements `StarlarkValue`, so `Starlark.fromJava`
/// passes it through unchanged.
// PORT: Java checks `x instanceof StarlarkValue`. Rust option values are type-erased, so the
// option value classes that implement StarlarkValue in Bazel 9.3.2 are listed here.
fn is_java_starlark_value(v: &OptionValue) -> bool {
    use blaze_optdefs::fragment::android_configuration as android;
    use blaze_optdefs::fragment::apple_configuration as apple;
    use blaze_optdefs::fragment::compilation_mode::CompilationMode;
    use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode;
    use blaze_optdefs::fragment::core_options as core;
    use blaze_optdefs::fragment::cpp_configuration as cpp;
    use blaze_optdefs::fragment::dotted_version::DottedVersionOption;
    use blaze_optdefs::fragment::java_configuration as java;
    use blaze_optdefs::fragment::python_version::PythonVersion;
    v.downcast_ref::<CompilationMode>().is_some()
        || v.downcast_ref::<blaze_options::TriState>().is_some()
        || v.downcast_ref::<core::CoreOptionsExecConfigurationDistinguisherScheme>()
            .is_some()
        || v.downcast_ref::<core::CoreOptionsOutputDirectoryNamingScheme>()
            .is_some()
        || v.downcast_ref::<core::CoreOptionsOutputPathsMode>()
            .is_some()
        || v.downcast_ref::<core::CoreOptionsIncludeConfigFragmentsEnum>()
            .is_some()
        || v.downcast_ref::<CoreOptionConvertersStrictDepsMode>()
            .is_some()
        || v.downcast_ref::<cpp::CppConfigurationDynamicMode>()
            .is_some()
        || v.downcast_ref::<cpp::CppConfigurationStripMode>().is_some()
        || v.downcast_ref::<java::JavaConfigurationJavaClasspathMode>()
            .is_some()
        || v.downcast_ref::<java::JavaConfigurationOneVersionEnforcementLevel>()
            .is_some()
        || v.downcast_ref::<PythonVersion>().is_some()
        || v.downcast_ref::<android::AndroidConfigurationAndroidManifestMerger>()
            .is_some()
        || v.downcast_ref::<android::AndroidConfigurationManifestMergerOrder>()
            .is_some()
        || v.downcast_ref::<android::AndroidConfigurationConfigurationDistinguisher>()
            .is_some()
        || v.downcast_ref::<apple::AppleConfigurationConfigurationDistinguisher>()
            .is_some()
        || v.downcast_ref::<DottedVersionOption>().is_some()
}

/// A Java object of a `StarlarkValue` class with no Starlark port of its own, carried through
/// Starlark unchanged (transition settings such as `CompilationMode`).
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaObjectValue(#[allocative(skip)] pub OptionValue);

starlark_simple_value!(JavaObjectValue);

impl fmt::Display for JavaObjectValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java prints these with their `repr` (Printer.debugPrint -> toString for enums).
        f.write_str(&self.0.java_to_string())
    }
}

#[starlark_value(type = "unknown")]
impl<'v> StarlarkValue<'v> for JavaObjectValue {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<JavaObjectValue>()
            .is_some_and(|o| o.0 == self.0))
    }
}

/// `Starlark.fromJava(x, mutability)`: `None` when the object is not Starlark-convertible
/// (`InvalidStarlarkValueException`).
fn from_java<'v>(x: &JavaObject, heap: starlark::values::Heap<'v>) -> Option<Value<'v>> {
    match x {
        JavaObject::Null => Some(Value::new_none()),
        JavaObject::Bool(b) => Some(Value::new_bool(*b)),
        JavaObject::Str(s) => Some(heap.alloc(s.as_str())),
        JavaObject::Label(l) => Some(heap.alloc(LabelValue(l.clone()))),
        JavaObject::StarlarkValue(v) => Some(heap.alloc(JavaObjectValue(v.clone()))),
        JavaObject::Int(i) => Some(heap.alloc(*i)),
        JavaObject::Float(f) => Some(heap.alloc(starlark::values::float::StarlarkFloat(*f))),
        JavaObject::List(l) => {
            // StarlarkList.copyOf checks each element with Starlark.checkValid.
            let mut elems = Vec::with_capacity(l.len());
            for e in l {
                elems.push(check_valid(e, heap)?);
            }
            Some(heap.alloc(AllocList(elems)))
        }
        // A Tuple is a StarlarkValue: Starlark.fromJava returns it unchanged.
        JavaObject::Tuple(l) => {
            let mut elems = Vec::with_capacity(l.len());
            for e in l {
                elems.push(from_java(e, heap)?);
            }
            Some(heap.alloc(starlark::values::tuple::AllocTuple(elems)))
        }
        JavaObject::Map(m) => {
            let mut entries = Vec::with_capacity(m.len());
            for (k, v) in m {
                entries.push((check_valid(k, heap)?, check_valid(v, heap)?));
            }
            Some(heap.alloc(AllocDict(entries)))
        }
        JavaObject::Other(_) => None,
    }
}

/// Whether `Starlark.fromJava` converts `x` (no `InvalidStarlarkValueException`).
fn is_from_java_convertible(x: &JavaObject) -> bool {
    match x {
        JavaObject::Null
        | JavaObject::Bool(_)
        | JavaObject::Str(_)
        | JavaObject::Label(_)
        | JavaObject::StarlarkValue(_)
        | JavaObject::Int(_)
        | JavaObject::Float(_) => true,
        JavaObject::List(l) => l.iter().all(is_valid),
        JavaObject::Tuple(l) => l.iter().all(is_from_java_convertible),
        JavaObject::Map(m) => m.iter().all(|(k, v)| is_valid(k) && is_valid(v)),
        JavaObject::Other(_) => false,
    }
}

/// `Starlark.valid`: String, Boolean and StarlarkValue objects.
fn is_valid(x: &JavaObject) -> bool {
    matches!(
        x,
        JavaObject::Bool(_)
            | JavaObject::Str(_)
            | JavaObject::Label(_)
            | JavaObject::StarlarkValue(_)
            | JavaObject::Tuple(_)
    )
}

/// `Starlark.checkValid`: only String, Boolean and StarlarkValue objects.
fn check_valid<'v>(x: &JavaObject, heap: starlark::values::Heap<'v>) -> Option<Value<'v>> {
    match x {
        JavaObject::Bool(_)
        | JavaObject::Str(_)
        | JavaObject::Label(_)
        | JavaObject::StarlarkValue(_)
        | JavaObject::Tuple(_)
        | JavaObject::Null => {
            if matches!(x, JavaObject::Null) {
                return None;
            }
            from_java(x, heap)
        }
        _ => None,
    }
}

/// Converts a Starlark value returned by a transition function to the Java object Bazel sees.
fn to_java_object(v: Value) -> Result<JavaObject, String> {
    if v.is_none() {
        return Ok(JavaObject::Null);
    }
    if let Some(b) = v.unpack_bool() {
        return Ok(JavaObject::Bool(b));
    }
    if let Some(i) = v.unpack_i32() {
        return Ok(JavaObject::Int(i as i64));
    }
    if let Some(s) = v.unpack_str() {
        return Ok(JavaObject::Str(s.to_string()));
    }
    if let Some(l) = v.downcast_ref::<LabelValue>() {
        return Ok(JavaObject::Label(l.0.clone()));
    }
    if let Some(o) = v.downcast_ref::<JavaObjectValue>() {
        return Ok(JavaObject::StarlarkValue(o.0.clone()));
    }
    if let Some(list) = ListRef::from_value(v) {
        return Ok(JavaObject::List(
            list.iter().map(to_java_object).collect::<Result<_, _>>()?,
        ));
    }
    if let Some(tuple) = TupleRef::from_value(v) {
        return Ok(JavaObject::Tuple(
            tuple
                .content()
                .iter()
                .map(|e| to_java_object(*e))
                .collect::<Result<_, _>>()?,
        ));
    }
    if let Some(dict) = DictRef::from_value(v) {
        let mut entries = Vec::new();
        for (k, val) in dict.iter() {
            entries.push((to_java_object(k)?, to_java_object(val)?));
        }
        return Ok(JavaObject::Map(entries));
    }
    Err(v.get_type().to_string())
}

// -------------------------------------------------------------------------------------------
// The Starlark transition object.

/// Which `StarlarkDefinedConfigTransition` subclass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Allocative)]
pub enum TransitionKind {
    /// `RegularTransition` (`transition()`).
    Regular,
    /// `ExecTransition` (`exec_transition()`).
    Exec,
}

/// The data of a Starlark transition that is not a Starlark value.
#[derive(Debug, Allocative)]
pub struct TransitionData {
    pub kind: TransitionKind,
    #[allocative(skip)]
    pub inputs_canonicalized_to_given: Vec<(Label, String)>,
    #[allocative(skip)]
    pub outputs_canonicalized_to_given: Vec<(Label, String)>,
    #[allocative(skip)]
    pub parent_label: Label,
    #[allocative(skip)]
    pub location: Option<Location>,
    #[allocative(skip)]
    pub package_context: LabelPackageContext,
    #[allocative(skip)]
    pub repo_mapping: RepositoryMapping,
}

/// The Starlark value `transition()`/`exec_transition()` return (Java
/// `StarlarkDefinedConfigTransition` as a `ConfigurationTransitionApi`).
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct TransitionObjectGen<V: starlark::values::ValueLifetimeless> {
    implementation: V,
    #[trace(static)]
    data: Arc<TransitionData>,
}

starlark_complex_value!(pub TransitionObject);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<TransitionObjectGen<Value<'v>>> for TransitionObjectGen<FrozenValue> {}

impl<'v> Freeze for TransitionObject<'v> {
    type Frozen = FrozenTransitionObject;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenTransitionObject> {
        Ok(TransitionObjectGen {
            implementation: self.implementation.freeze(freezer)?,
            data: self.data,
        })
    }
}

impl<V: starlark::values::ValueLifetimeless> fmt::Display for TransitionObjectGen<V> {
    // Java: StarlarkDefinedConfigTransition.RegularTransition.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<transition object>")
    }
}

#[starlark_value(type = "transition")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for TransitionObjectGen<V> where
    Self: ProvidesStaticType<'v>
{
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:133 StarlarkDefinedConfigTransition.canonicalizeSetting
/// Returns a build settings in canonicalized form taking into account repository remappings.
/// Native options only have one form so they are always returned unchanged (i.e.
/// `//command_line_option:<option-name>`).
fn canonicalize_setting(
    setting: &str,
    package_context: &LabelPackageContext,
) -> Result<Label, blaze_cmdline::LabelSyntaxException> {
    // native options
    if setting.starts_with(COMMAND_LINE_OPTION_PREFIX) {
        return Label::parse_canonical(setting);
    }
    Label::parse_with_package_context(setting, package_context)
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:114 StarlarkDefinedConfigTransition.getCanonicalizedSettings
/// Canonicalize the given list of settings. Return a map of their canonicalized version to the
/// form they were given in.
fn get_canonicalized_settings(
    package_context: &LabelPackageContext,
    settings: &[String],
    disallowed_options: &[String],
    inputs_or_outputs: &str,
) -> Result<Vec<(Label, String)>, String> {
    let mut canonicalized_to_given: Vec<(Label, String)> = Vec::new();
    for setting in settings {
        let canonicalized_setting =
            canonicalize_setting(setting, package_context).map_err(|_| {
                format!(
                    "Malformed label in transition {} parameter: '{}'",
                    inputs_or_outputs, setting
                )
            })?;
        if canonicalized_setting.get_package_identifier()
            == &*COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER
        {
            let option_name = canonicalized_setting.get_name();
            if disallowed_options.iter().any(|o| o == option_name) {
                return Err(format!(
                    "Option '{}' is not allowed in transitions {}.",
                    option_name, inputs_or_outputs
                ));
            }
        }
        if let Some((_, previous_setting)) = canonicalized_to_given
            .iter()
            .find(|(l, _)| *l == canonicalized_setting)
        {
            return Err(format!(
                "Transition declares duplicate build setting '{}' in {} (specified as '{}' and '{}')",
                canonicalized_setting, inputs_or_outputs, setting, previous_setting
            ));
        }
        canonicalized_to_given.push((canonicalized_setting, setting.clone()));
    }
    // ImmutableSortedMap.copyOf: natural Label order.
    canonicalized_to_given.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(canonicalized_to_given)
}

// Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:60 ConfigGlobalLibrary.validateBuildSettingKeys
fn validate_build_setting_keys(
    option_keys: &[String],
    inputs: bool,
    allow_incompatible_and_experimental_options: bool,
    package_context: &LabelPackageContext,
) -> Result<(), String> {
    let mut processed_options: Vec<&String> = Vec::new();
    let singular_error_descriptor = if inputs { "input" } else { "output" };
    for option_key in option_keys {
        if !option_key.starts_with(COMMAND_LINE_OPTION_PREFIX) {
            match Label::parse_with_package_context(option_key, package_context) {
                Ok(label) => {
                    if !label.get_repository().is_visible() {
                        blaze_util::unported!(
                            "ConfigGlobalLibrary.validateBuildSettingKeys",
                            "invisible repo in transition setting {}",
                            label
                        );
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "invalid transition {} '{}'. If this is intended as a native option, it must begin with //command_line_option: {}",
                        singular_error_descriptor,
                        option_key,
                        e.get_message()
                    ));
                }
            }
        } else {
            let option_name = &option_key[COMMAND_LINE_OPTION_PREFIX.len()..];
            if !allow_incompatible_and_experimental_options && !valid_option_name(option_name) {
                return Err(format!(
                    "Invalid transition {} '{}'. Cannot transition on --experimental_* or --incompatible_* options",
                    singular_error_descriptor, option_key
                ));
            }
        }
        if processed_options.contains(&option_key) {
            return Err(format!(
                "duplicate transition {} '{}'",
                singular_error_descriptor, option_key
            ));
        }
        processed_options.push(option_key);
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:193 ConfigGlobalLibrary.validOptionName
/// Flags that user-defined transitions aren't allowed to set.
fn valid_option_name(option_name: &str) -> bool {
    if option_name.starts_with("experimental_") {
        // Don't allow experimental flags.
        return false;
    }
    if option_name == "incompatible_enable_cc_toolchain_resolution"
        || option_name == "incompatible_enable_apple_toolchain_resolution"
    {
        // This is specifically allowed.
        return true;
    } else if option_name.starts_with("incompatible_") {
        // Don't allow other incompatible flags.
        return false;
    }
    true
}

fn string_list(v: Value, what: &str) -> anyhow::Result<Vec<String>> {
    // Java: Sequence.cast(inputs, String.class, "inputs")
    let items: Vec<Value> = if let Some(l) = ListRef::from_value(v) {
        l.iter().collect()
    } else if let Some(t) = TupleRef::from_value(v) {
        t.content().to_vec()
    } else {
        anyhow::bail!("for {}, got {}, want sequence", what, v.get_type());
    };
    items
        .into_iter()
        .enumerate()
        .map(|(i, x)| {
            x.unpack_str().map(str::to_owned).ok_or_else(|| {
                anyhow::anyhow!(
                    "at index {} of {}, got element of type {}, want string",
                    i,
                    what,
                    x.get_type()
                )
            })
        })
        .collect()
}

/// The data `exec_transition()`/`transition()` need from the calling .bzl file
/// (`BazelModuleContext`) and the semantics; set as the thread payload.
pub struct TransitionDefinitionContext {
    pub module_label: Label,
    pub repo_mapping: RepositoryMapping,
    /// `--incompatible_disable_transitions_on`.
    pub disallowed_options: Vec<String>,
}

// Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:83 ConfigGlobalLibrary.execTransition
fn new_transition<'v>(
    kind: TransitionKind,
    implementation: Value<'v>,
    inputs: Value<'v>,
    outputs: Value<'v>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> anyhow::Result<Value<'v>> {
    let location = blaze_starlark::env::caller_location(eval);
    let cx = ThreadContext::get(eval)?;
    let (module_label, repo_mapping, disallowed_options) = {
        let payload = cx
            .payload::<TransitionDefinitionContext>()
            .ok_or_else(|| anyhow::anyhow!("transitions can only be defined in .bzl files"))?;
        (
            payload.module_label.clone(),
            payload.repo_mapping.clone(),
            payload.disallowed_options.clone(),
        )
    };
    if kind == TransitionKind::Exec {
        // Java: BuiltinRestriction.failIfCalledOutsideBuiltins(thread)
        if module_label.get_repository() != &*blaze_cmdline::repository_name::BUILTINS {
            anyhow::bail!("{} is a private API", "exec_transition");
        }
    }
    let package_context =
        LabelPackageContext::of(module_label.get_package_identifier(), repo_mapping.clone());
    let inputs_list = string_list(inputs, "inputs")?;
    let outputs_list = string_list(outputs, "outputs")?;
    let allow_incompatible = kind == TransitionKind::Exec;
    validate_build_setting_keys(&inputs_list, true, allow_incompatible, &package_context)
        .map_err(|e| anyhow::anyhow!(e))?;
    validate_build_setting_keys(&outputs_list, false, allow_incompatible, &package_context)
        .map_err(|e| anyhow::anyhow!(e))?;
    // Java: StarlarkDefinedConfigTransition.<init> (outputs first, then inputs).
    let outputs_canonicalized_to_given = get_canonicalized_settings(
        &package_context,
        &outputs_list,
        &disallowed_options,
        "OUTPUTS",
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let inputs_canonicalized_to_given = get_canonicalized_settings(
        &package_context,
        &inputs_list,
        &disallowed_options,
        "INPUTS",
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(eval.heap().alloc(TransitionObjectGen {
        implementation,
        data: Arc::new(TransitionData {
            kind,
            inputs_canonicalized_to_given,
            outputs_canonicalized_to_given,
            parent_label: module_label,
            location,
            package_context,
            repo_mapping,
        }),
    }))
}

/// `_builtins.toplevel.exec_transition` (Java `ConfigGlobalLibrary.execTransition`).
#[starlark_module]
fn builtins_toplevel(builder: &mut GlobalsBuilder) {
    fn exec_transition<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] inputs: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        new_transition(TransitionKind::Exec, implementation, inputs, outputs, eval)
    }
}

/// `_builtins.internal.py_builtins` (Java `PyBuiltins`): only what the exec transition needs.
#[starlark_module]
fn py_builtins(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:116 PyBuiltins.getCurrentOsName
    /// Get the name of the OS Bazel itself is running on.
    fn get_current_os_name() -> anyhow::Result<String> {
        // PORT: Linux only (OS.LINUX.getCanonicalName()).
        Ok("linux".to_owned())
    }
}

/// A builtin symbol that is not ported; any use of it fails loudly.
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
struct UnportedBuiltin(&'static str);

starlark_simple_value!(UnportedBuiltin);

impl fmt::Display for UnportedBuiltin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<unported {}>", self.0)
    }
}

#[starlark_value(type = "unported")]
impl<'v> StarlarkValue<'v> for UnportedBuiltin {
    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &starlark::eval::Arguments<'v, '_>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        blaze_util::unported!(self.0)
    }
}

fn builtins_globals(semantics: &StarlarkSemantics) -> starlark::environment::Globals {
    let mut b = blaze_starlark::env::universe_builder(semantics);
    blaze_starlark::structs::register_struct(&mut b);
    blaze_starlark::label::register_label(&mut b);
    b.namespace("_builtins", |nb| {
        nb.namespace("toplevel", |tb| {
            builtins_toplevel(tb);
            tb.set(
                "PackageSpecificationInfo",
                UnportedBuiltin("PackageSpecificationInfo"),
            );
        });
        nb.namespace("internal", |ib| {
            ib.namespace("py_builtins", py_builtins);
        });
    });
    b.build()
}

/// Events from builtins evaluation go to a handler at the call site; there are none in practice.
struct NoEvents;

impl StarlarkEvents for NoEvents {
    fn debug(&self, _location: Option<&Location>, _message: &str) {}
    fn warn(&self, _location: Option<&Location>, _message: &str) {}
}

/// The bundled builtins .bzl files this module evaluates (copies of
/// `src/main/starlark/builtins_bzl` at the pin).
const BUILTIN_EXEC_PLATFORMS_BZL: &str =
    include_str!("../builtins_bzl/common/builtin_exec_platforms.bzl");
const PY_INTERNAL_BZL: &str = include_str!("../builtins_bzl/common/python/py_internal.bzl");

fn exec_builtins_bzl(
    label: &Label,
    source: &str,
    loads: &blaze_starlark::loader::LoadMap,
    semantics: &Arc<StarlarkSemantics>,
) -> Result<FrozenModule, EvalError> {
    let file_name = label.get_canonical_form();
    let compiled =
        blaze_starlark::eval::compile(FileKind::BuiltinsBzl, &file_name, source.to_owned())?;
    let globals = builtins_globals(semantics);
    let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();
    blaze_starlark::eval::run_on_evaluation_thread(|| {
        let ctx = ThreadContext::new(Phase::BzlLoad, semantics.clone(), Arc::new(NoEvents))
            .with_file_label(label.clone())
            .with_payload(TransitionDefinitionContext {
                module_label: label.clone(),
                repo_mapping: mapping,
                disallowed_options: semantics.incompatible_disable_transitions_on.clone(),
            });
        blaze_starlark::eval::exec_module(
            compiled,
            &globals,
            loads,
            &ctx,
            &blaze_starlark::eval::EvalOptions::default(),
        )
    })
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkExecTransitionLoader.java:38 StarlarkExecTransitionLoader.StarlarkExecTransitionLoadingException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StarlarkExecTransitionLoadingException(pub String);

impl StarlarkExecTransitionLoadingException {
    pub fn with_context(context: &str, reference: &str, message: &str) -> Self {
        StarlarkExecTransitionLoadingException(format!(
            "Bad Starlark transition reference from {}: {}. {}.",
            context, reference, message
        ))
    }
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkExecTransitionLoader.java:78 StarlarkExecTransitionLoader.loadStarlarkExecTransition
/// Loads the Starlark transition that implements execution transition logic according to
/// `CoreOptions.starlarkExecConfig`. Returns `None` if the build implements the exec transition
/// with native logic (empty options).
pub fn load_starlark_exec_transition(
    options: Option<&crate::build_options::BuildOptions>,
    semantics: &Arc<StarlarkSemantics>,
) -> Result<Option<Arc<StarlarkDefinedConfigTransition>>, StarlarkExecTransitionLoadingException> {
    let Some(options) = options else {
        return Ok(None);
    };
    if *options == *crate::transitions::EMPTY_OPTIONS {
        return Ok(None);
    }
    let user_ref = options
        .get::<blaze_optdefs::fragment::core_options::CoreOptions>()
        .expect("CoreOptions")
        .starlark_exec_config
        .clone();
    let flag_name = "--experimental_exec_config";
    let (bzl_file, symbol) = transition_reference_create(&user_ref, flag_name)?;
    if bzl_file.get_repository() != &*blaze_cmdline::repository_name::BUILTINS {
        // PORT: BzlLoadValue.keyForBuild needs BZL_LOAD (blaze_sky_loading).
        blaze_util::unported!(
            "StarlarkExecTransitionLoader.loadStarlarkExecTransition",
            "--experimental_exec_config outside @_builtins: {}",
            user_ref
        );
    }
    let module = load_builtins_bzl(&bzl_file, semantics).map_err(|e| {
        StarlarkExecTransitionLoadingException::with_context(flag_name, &user_ref, &e)
    })?;
    let transition = module.get(&symbol).map_err(|_| {
        StarlarkExecTransitionLoadingException::with_context(
            flag_name,
            &user_ref,
            &format!("{} not found in {}", symbol, bzl_file),
        )
    })?;
    match StarlarkDefinedConfigTransition::from_frozen(transition) {
        Some(t) => Ok(Some(Arc::new(t))),
        None => Err(StarlarkExecTransitionLoadingException::with_context(
            flag_name,
            &user_ref,
            &format!("{} is not a Starlark transition", symbol),
        )),
    }
}

/// Loads one of the bundled `@_builtins` files (and its loads).
fn load_builtins_bzl(
    label: &Label,
    semantics: &Arc<StarlarkSemantics>,
) -> Result<FrozenModule, String> {
    let exec_platforms = Label::parse_canonical("@_builtins//:common/builtin_exec_platforms.bzl")
        .expect("valid label");
    if *label != exec_platforms {
        // PORT: only the default exec transition file is bundled here.
        blaze_util::unported!(
            "BzlLoadFunction.computeInternal",
            "@_builtins file {}",
            label
        );
    }
    let py_internal_label =
        Label::parse_canonical("@_builtins//:common/python/py_internal.bzl").expect("valid label");
    let py_internal = exec_builtins_bzl(
        &py_internal_label,
        PY_INTERNAL_BZL,
        &blaze_starlark::loader::LoadMap::new(),
        semantics,
    )
    .map_err(|e| e.to_string())?;
    let mut loads = blaze_starlark::loader::LoadMap::new();
    loads.insert("@_builtins//:common/python/py_internal.bzl", py_internal);
    exec_builtins_bzl(
        &exec_platforms,
        BUILTIN_EXEC_PLATFORMS_BZL,
        &loads,
        semantics,
    )
    .map_err(|e| e.to_string())
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkExecTransitionLoader.java:154 TransitionReference.create
/// Returns a structured form of a user-specified Starlark transition reference
/// (`//pkg:file.bzl%symbol`).
fn transition_reference_create(
    user_ref: &str,
    context: &str,
) -> Result<(Label, String), StarlarkExecTransitionLoadingException> {
    let splitval: Vec<&str> = user_ref.split('%').collect();
    if splitval.len() < 2 || splitval[1].is_empty() {
        return Err(StarlarkExecTransitionLoadingException::with_context(
            context,
            user_ref,
            "Doesn't match expected form //pkg:file.bzl%%symbol",
        ));
    }
    match Label::parse_canonical(splitval[0]) {
        Ok(l) => Ok((l, splitval[1].to_string())),
        Err(e) => Err(StarlarkExecTransitionLoadingException::with_context(
            context,
            user_ref,
            &format!("Bad label {}: {}", splitval[0], e.get_message()),
        )),
    }
}

/// The `attr` argument of a transition implementation (Java: the transition's `attrObject`, a
/// struct of the target's attribute values), allocated on the evaluation's heap.
// W4-analysis: built by blaze_analysis from the configured attributes.
pub trait TransitionAttrObject: Send + Sync {
    fn alloc<'v>(&self, heap: starlark::values::Heap<'v>) -> Value<'v>;
}

/// The empty `attr` struct (the exec transition and the baseline computation).
// W4-analysis: see TransitionAttrObject.
pub struct EmptyTransitionAttrs;

impl TransitionAttrObject for EmptyTransitionAttrs {
    fn alloc<'v>(&self, heap: starlark::values::Heap<'v>) -> Value<'v> {
        heap.alloc(blaze_starlark::structs::StarlarkInfo::new(
            heap.alloc(blaze_starlark::structs::StructProvider),
            Vec::new(),
        ))
    }
}

// -------------------------------------------------------------------------------------------
// StarlarkDefinedConfigTransition.

// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:75 StarlarkDefinedConfigTransition
/// A configuration transition defined in Starlark (frozen).
pub struct StarlarkDefinedConfigTransition {
    /// The frozen `TransitionObject`.
    object: OwnedFrozenValue,
    /// The frozen implementation function.
    implementation: OwnedFrozenValue,
    data: Arc<TransitionData>,
}

impl fmt::Debug for StarlarkDefinedConfigTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StarlarkDefinedConfigTransition({:?})", self.data)
    }
}

/// `StarlarkDefinedConfigTransition.ValidationException`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationException(pub String);

impl StarlarkDefinedConfigTransition {
    /// The transition held by a frozen Starlark value, if it is one.
    pub fn from_frozen(value: OwnedFrozenValue) -> Option<Self> {
        let data = value
            .value()
            .downcast_ref::<FrozenTransitionObject>()?
            .data
            .clone();
        let implementation = value.map(|v| {
            v.downcast_ref::<FrozenTransitionObject>()
                .expect("checked")
                .implementation
        });
        Some(StarlarkDefinedConfigTransition {
            object: value,
            implementation,
            data,
        })
    }

    /// A transition from its parts, for Starlark transitions created by another `transition()`
    /// implementation (the loading phase's `blaze_config` value): `object` is the frozen
    /// transition value, `implementation` its frozen implementation function.
    // W4-analysis: added for applying rule/attribute transitions (crates/blaze_analysis/DESIGN.md).
    pub fn from_parts(
        object: OwnedFrozenValue,
        implementation: OwnedFrozenValue,
        data: TransitionData,
    ) -> Self {
        StarlarkDefinedConfigTransition {
            object,
            implementation,
            data: Arc::new(data),
        }
    }

    pub fn object(&self) -> &OwnedFrozenValue {
        &self.object
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:126 StarlarkDefinedConfigTransition.isExecTransition
    pub fn is_exec_transition(&self) -> bool {
        self.data.kind == TransitionKind::Exec
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:192 StarlarkDefinedConfigTransition.isForAnalysisTesting
    pub fn is_for_analysis_testing(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:198 StarlarkDefinedConfigTransition.getInputs
    /// Returns the given input option keys for this transition.
    pub fn get_inputs(&self) -> Vec<String> {
        self.data
            .inputs_canonicalized_to_given
            .iter()
            .map(|(_, g)| g.clone())
            .collect()
    }

    pub fn get_inputs_canonicalized_to_given(&self) -> &[(Label, String)] {
        &self.data.inputs_canonicalized_to_given
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:210 StarlarkDefinedConfigTransition.getOutputs
    /// Returns the given output option keys for this transition.
    pub fn get_outputs(&self) -> Vec<String> {
        self.data
            .outputs_canonicalized_to_given
            .iter()
            .map(|(_, g)| g.clone())
            .collect()
    }

    pub fn get_outputs_canonicalized_to_given(&self) -> &[(Label, String)] {
        &self.data.outputs_canonicalized_to_given
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:219 StarlarkDefinedConfigTransition.getLocation
    pub fn get_location(&self) -> Option<&Location> {
        self.data.location.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:121 StarlarkDefinedConfigTransition.getPackageContext
    pub fn get_package_context(&self) -> &LabelPackageContext {
        &self.data.package_context
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:455 RegularTransition.getTransitionSafeString
    /// Converts a Java-native flag value to a Starlark-readable string, or fails if the flag's
    /// type can't be represented in Starlark.
    fn get_transition_safe_string(
        name: &str,
        value: &JavaObject,
        option_info_map: &IndexMap<String, OptionInfo>,
    ) -> Result<JavaObject, String> {
        let raw = match value {
            JavaObject::Other(v) | JavaObject::StarlarkValue(v) => Some(v),
            _ => None,
        };
        if let Some(v) = raw {
            if let Some(r) = v.downcast_ref::<blaze_optdefs::fragment::regex_filter::RegexFilter>()
            {
                // RegExFilter doesn't serialize to the same value it originally had on the
                // command line. Call toOriginalString, to do that properly.
                return Ok(JavaObject::Str(
                    r.to_original_string().expect("verifyNotNull").to_string(),
                ));
            }
            if let Some(p) = v.downcast_ref::<blaze_options::Foreign<blaze_vfs::PathFragment>>() {
                // Starlark#fromJava doesn't understand this Bazel-specific Java type. But its
                // toString() method serializes cleanly.
                return Ok(JavaObject::Str(p.0.to_string()));
            }
        }
        // See if the option's converter knows how to produce to Starlark values.
        let option_def: OptionDefinitionRef = option_info_map
            .get(&name[COMMAND_LINE_OPTION_PREFIX.len()..])
            .expect("native option")
            .get_definition();
        let converter = option_def.get_converter();
        if !converter.starlark_convertible() {
            return Err(format!(
                "Input build setting {} is of type {}, which is unreadable in Starlark. Please submit a feature request.",
                name,
                java_class_of(value)
            ));
        }
        if option_def.allows_multiple() {
            let JavaObject::List(as_list) = value else {
                panic!("ClassCastException: allowMultiple option value is not a List");
            };
            // If this is an empty list, Starlark#fromJava should have handled it.
            blaze_util::check_state!(!as_list.is_empty());
            blaze_util::check_state!(!matches!(as_list[0], JavaObject::List(_)));
            Ok(JavaObject::List(
                as_list
                    .iter()
                    .map(|o| {
                        JavaObject::Str(
                            converter.reverse_for_starlark(
                                &o.to_option_value().expect("non-null element"),
                            ),
                        )
                    })
                    .collect(),
            ))
        } else {
            // This isn't allowMultiple, so the converter is a straightforward reversal.
            Ok(JavaObject::Str(converter.reverse_for_starlark(
                &value.to_option_value().expect("non-null value"),
            )))
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:265 RegularTransition.evaluate
    /// Given a map of a subset of the "previous" build settings, returns the changed build
    /// settings as a result of applying this transition. Returns `None` if errors were reported
    /// to the handler.
    pub fn evaluate(
        &self,
        previous_settings: &IndexMap<String, JavaObject>,
        option_info_map: &IndexMap<String, OptionInfo>,
        semantics: &Arc<StarlarkSemantics>,
        handler: &dyn EventHandler,
    ) -> Option<IndexMap<String, IndexMap<Label, JavaObject>>> {
        self.evaluate_with_attrs(
            previous_settings,
            option_info_map,
            &EmptyTransitionAttrs,
            semantics,
            handler,
        )
    }

    /// [`Self::evaluate`] with the `attr` object passed to the implementation.
    // W4-analysis: rule and attribute transitions read the target's attributes.
    pub fn evaluate_with_attrs(
        &self,
        previous_settings: &IndexMap<String, JavaObject>,
        option_info_map: &IndexMap<String, OptionInfo>,
        attr_object: &dyn TransitionAttrObject,
        semantics: &Arc<StarlarkSemantics>,
        handler: &dyn EventHandler,
    ) -> Option<IndexMap<String, IndexMap<Label, JavaObject>>> {
        // createBuildSettingsDict: values Starlark#fromJava can't read are converted first.
        let mut settings: Vec<(String, JavaObject, bool)> = Vec::new();
        for (key, value) in previous_settings {
            settings.push((key.clone(), value.clone(), false));
        }
        // Detect the values fromJava cannot convert.
        let convertible: Vec<bool> = settings
            .iter()
            .map(|(_, v, _)| is_from_java_convertible(v))
            .collect();
        for (i, ok) in convertible.into_iter().enumerate() {
            if !ok {
                let (key, value, _) = &settings[i];
                match Self::get_transition_safe_string(key, value, option_info_map) {
                    Ok(converted) => settings[i] = (key.clone(), converted, true),
                    Err(message) => {
                        // TODO(blaze-configurability-team): Ideally, the error would happen (and
                        // thus location) at the transition() call during loading phase.
                        handler.handle(&Event::error(format!(
                            "{}before calling {}: {}",
                            self.impl_location_prefix(),
                            self.impl_name(),
                            message
                        )));
                        return None;
                    }
                }
            }
        }

        // thread.setPrintHandler(Event.makeDebugPrintHandler(handler)): print() output of the
        // implementation function reaches `handler` as DEBUG events, before the error of a
        // failed call.
        // PORT: the events are collected while the function runs (the Starlark thread context
        // needs a 'static sink) and handed to `handler` in order right after the call returns.
        let print_events = Arc::new(blaze_starlark::thread::CollectingEvents::default());
        let result = blaze_starlark::eval::run_on_evaluation_thread(|| {
            let ctx = ThreadContext::new(Phase::Analysis, semantics.clone(), print_events.clone());
            blaze_starlark::eval::call_frozen(
                &self.implementation,
                &ctx,
                &blaze_starlark::eval::EvalOptions::default(),
                |module| {
                    let heap = module.heap();
                    let entries: Vec<(&str, Value)> = settings
                        .iter()
                        .map(|(k, v, _)| {
                            (
                                k.as_str(),
                                from_java(v, heap).expect("convertible after fallback"),
                            )
                        })
                        .collect();
                    let dict = heap.alloc(AllocDict(entries));
                    // The attributes of the target (empty for the exec transition).
                    let attr = attr_object.alloc(heap);
                    (vec![dict, attr], vec![])
                },
                |res, _module, _eval| Ok(Self::extract_result(res)),
            )
        });
        // Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
        for event in print_events.take() {
            let location = event.location.as_ref().map(|l| {
                blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32)
            });
            handler.handle(&match event.kind {
                "DEBUG" => Event::debug_at(location, event.message),
                "WARNING" => Event::warn_at(location, event.message),
                _ => Event::error_at(location, event.message),
            });
        }
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                handler.handle(&Event::error(e.to_string()));
                return None;
            }
        };
        match result {
            ExtractedResult::None => Some(IndexMap::new()),
            ExtractedResult::Dict(entries) => {
                if entries.is_empty() {
                    return Some(IndexMap::new());
                }
                // Try dict of dicts.
                let all_dicts = entries.iter().all(|(k, v)| {
                    matches!(k, JavaObject::Str(_)) && matches!(v, JavaObject::Map(_))
                });
                if all_dicts {
                    let mut builder = IndexMap::new();
                    for (k, v) in entries {
                        let (JavaObject::Str(k), JavaObject::Map(raw)) = (k, v) else {
                            unreachable!()
                        };
                        match self.canonicalize_transition_output_dict(&raw) {
                            Ok(d) => {
                                builder.insert(k, d);
                            }
                            Err(message) => {
                                self.errorf(
                                    handler,
                                    &format!(
                                        "invalid result from transition function: {}",
                                        message
                                    ),
                                );
                                return None;
                            }
                        }
                    }
                    return Some(builder);
                }
                // Try if this is a patch transition.
                // Java: net/starlark/java/eval/Dict.java:659 Dict.cast
                if let Some((k, v)) = entries
                    .iter()
                    .find(|(k, _)| !matches!(k, JavaObject::Str(_)))
                {
                    self.errorf(
                        handler,
                        &format!(
                            "invalid result from transition function: got dict<{}, {}> for 'dictionary of options', want dict<string, unknown>",
                            k.starlark_type(),
                            v.starlark_type()
                        ),
                    );
                    return None;
                }
                match self.canonicalize_transition_output_dict(&entries) {
                    Ok(d) => {
                        let mut m = IndexMap::new();
                        m.insert(PATCH_TRANSITION_KEY.to_string(), d);
                        Some(m)
                    }
                    Err(message) => {
                        self.errorf(
                            handler,
                            &format!("invalid result from transition function: {}", message),
                        );
                        None
                    }
                }
            }
            ExtractedResult::List(items) => {
                if items.is_empty() {
                    return Some(IndexMap::new());
                }
                let mut builder = IndexMap::new();
                // Sequence.cast(result, Dict.class, "dictionary of options dictionaries") checks
                // every element before any entry is canonicalized.
                // Java: net/starlark/java/eval/Sequence.java:142 Sequence.cast
                for (i, entry) in items.iter().enumerate() {
                    if !matches!(entry, JavaObject::Map(_)) {
                        self.errorf(
                            handler,
                            &format!(
                                "invalid result from transition function: at index {} of dictionary of options dictionaries, got element of type {}, want dict",
                                i,
                                entry.starlark_type()
                            ),
                        );
                        return None;
                    }
                }
                for (i, entry) in items.into_iter().enumerate() {
                    let JavaObject::Map(raw) = entry else {
                        unreachable!("checked by the cast above")
                    };
                    // Dict.cast(entry, String.class, Object.class, "dictionary of options")
                    // Java: net/starlark/java/eval/Dict.java:659 Dict.cast
                    if let Some((k, v)) = raw.iter().find(|(k, _)| !matches!(k, JavaObject::Str(_)))
                    {
                        self.errorf(
                            handler,
                            &format!(
                                "invalid result from transition function: got dict<{}, {}> for 'dictionary of options', want dict<string, unknown>",
                                k.starlark_type(),
                                v.starlark_type()
                            ),
                        );
                        return None;
                    }
                    match self.canonicalize_transition_output_dict(&raw) {
                        Ok(d) => {
                            builder.insert(i.to_string(), d);
                        }
                        Err(message) => {
                            self.errorf(
                                handler,
                                &format!("invalid result from transition function: {}", message),
                            );
                            return None;
                        }
                    }
                }
                Some(builder)
            }
            ExtractedResult::Other(t) => {
                self.errorf(
                    handler,
                    &format!(
                        "transition function returned {}, want dict or list of dicts",
                        t
                    ),
                );
                None
            }
            ExtractedResult::Unconvertible(t) => {
                blaze_util::unported!(
                    "StarlarkDefinedConfigTransition.evaluate",
                    "transition function returned a value of type {}",
                    t
                )
            }
        }
    }

    fn extract_result(res: Value) -> ExtractedResult {
        if res.downcast_ref::<NoneType>().is_some() {
            return ExtractedResult::None;
        }
        if let Some(dict) = DictRef::from_value(res) {
            let mut entries = Vec::new();
            for (k, v) in dict.iter() {
                match (to_java_object(k), to_java_object(v)) {
                    (Ok(k), Ok(v)) => entries.push((k, v)),
                    (Err(t), _) | (_, Err(t)) => return ExtractedResult::Unconvertible(t),
                }
            }
            return ExtractedResult::Dict(entries);
        }
        if ListRef::from_value(res).is_some() || TupleRef::from_value(res).is_some() {
            return match to_java_object(res) {
                Ok(JavaObject::List(items)) | Ok(JavaObject::Tuple(items)) => {
                    ExtractedResult::List(items)
                }
                Ok(_) => unreachable!(),
                Err(t) => ExtractedResult::Unconvertible(t),
            };
        }
        ExtractedResult::Other(res.get_type().to_string())
    }

    /// `impl.getLocation()` as an event location prefix: where the implementation function is
    /// declared (the name of its `def`), not where `transition()` was called.
    // Java: net/starlark/java/eval/StarlarkFunction.java:172 StarlarkFunction.getLocation
    fn impl_location_prefix(&self) -> String {
        let impl_location = blaze_starlark::defining_module::DefModule::declaration_span(
            self.implementation.value(),
        )
        .map(|span| Location::of_span_start(&span));
        match impl_location {
            Some(l) => format!("{}: ", l),
            // Java: a callable that is not a StarlarkFunction has Location.BUILTIN.
            // Java: net/starlark/java/syntax/Location.java:114 Location.BUILTIN
            None => "<builtin>: ".to_owned(),
        }
    }

    fn impl_name(&self) -> String {
        // PORT: Java names the implementation function (`StarlarkCallable.getName`).
        self.implementation.value().to_str()
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:602 RegularTransition.errorf
    fn errorf(&self, handler: &dyn EventHandler, message: &str) {
        handler.handle(&Event::error(format!(
            "{}{}",
            self.impl_location_prefix(),
            message
        )));
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:667 RegularTransition.validateFunctionOutputsMatchesDeclaredOutputs
    /// Validates that function outputs exactly the set of outputs it declares.
    fn validate_function_outputs_matches_declared_outputs(
        returned_key_set: &[String],
        declared_return_settings: &[String],
    ) -> Result<(), String> {
        if declared_return_settings
            .iter()
            .all(|d| returned_key_set.contains(d))
            && returned_key_set.len() == declared_return_settings.len()
        {
            return Ok(());
        }
        let mut remaining_outputs: Vec<&String> = declared_return_settings.iter().collect();
        for output_key in returned_key_set {
            match remaining_outputs.iter().position(|o| *o == output_key) {
                Some(i) => {
                    remaining_outputs.remove(i);
                }
                None => {
                    return Err(format!(
                        "transition function returned undeclared output '{}'",
                        output_key
                    ));
                }
            }
        }
        if !remaining_outputs.is_empty() {
            return Err(format!(
                "transition outputs [{}] were not defined by transition function",
                remaining_outputs
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:597 RegularTransition.canonicalizeTransitionOutputDict
    /// Given a map of build settings to their values, return a map with the same build settings
    /// but in their canonicalized string form to their values.
    fn canonicalize_transition_output_dict(
        &self,
        dict: &[(JavaObject, JavaObject)],
    ) -> Result<IndexMap<Label, JavaObject>, String> {
        let keys: Vec<String> = dict
            .iter()
            .map(|(k, _)| match k {
                JavaObject::Str(s) => s.clone(),
                other => other.java_to_string(),
            })
            .collect();
        Self::validate_function_outputs_matches_declared_outputs(&keys, &self.get_outputs())?;

        let mut canonicalized_to_given: Vec<(Label, String)> = Vec::new();
        let mut canonicalized_dict: Vec<(Label, JavaObject)> = Vec::new();
        for (returned_setting, (_, value)) in keys.iter().zip(dict.iter()) {
            let label = canonicalize_setting(returned_setting, &self.data.package_context)
                .map_err(|_| {
                    format!(
                        "Malformed label in transition return dictionary: '{}'",
                        returned_setting
                    )
                })?;
            if let Some((_, previous_given)) =
                canonicalized_to_given.iter().find(|(l, _)| *l == label)
            {
                return Err(format!(
                    "Transition implementation function returns the same option '{}' in two different keys: '{}' and '{}'",
                    label, returned_setting, previous_given
                ));
            }
            canonicalized_to_given.push((label.clone(), returned_setting.clone()));
            canonicalized_dict.push((label, value.clone()));
        }
        // ImmutableSortedMap with natural Label ordering.
        canonicalized_dict.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(canonicalized_dict.into_iter().collect())
    }
}

enum ExtractedResult {
    None,
    Dict(Vec<(JavaObject, JavaObject)>),
    List(Vec<JavaObject>),
    Other(String),
    Unconvertible(String),
}

/// `value.getClass()` for the unreadable-setting error.
fn java_class_of(value: &JavaObject) -> String {
    match value {
        JavaObject::Other(v) | JavaObject::StarlarkValue(v) => {
            // PORT: Java prints `class <binary name>`; Rust has the Rust type only.
            format!("class {:?}", v)
        }
        other => format!("class {}", other.starlark_type()),
    }
}
