//! Port of `analysis/starlark/StarlarkAttrModule.java`: the `attr` module and its
//! `Descriptor` (Starlark type `Attribute`).

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::starlark_value;

use super::aspect::aspect_info_of;
use super::bzl_globals::late_bound_default_of;
use super::config::convert_cfg;
use super::provider::provider_view;
use super::util::errorf;
use super::util::eval_exception;
use super::util::label_converter_for_bzl;
use super::util::sequence_cast;
use super::util::sequence_cast_strings;
use super::util::sequence_elements;
use super::util::sequence_noneable_cast;
use super::util::sequence_noneable_cast_strings;
use super::util::starlark_type;
use super::util::trim_doc_string;
use super::value_cell::PendingValue;
use crate::attribute::AllowedValueSet;
use crate::attribute::Attribute;
use crate::attribute::AttributeBuilder;
use crate::attribute::AttributeValueSource;
use crate::attribute::ImmutableAttributeFactory;
use crate::attribute::StarlarkComputedDefaultTemplate;
use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::bzl_context::BzlInitThreadContext;
use crate::package::TargetDefinitionContext;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::types::AttrValue;
use crate::types::FileType;
use crate::types::FileTypeSet;
use crate::types::LabelClass;
use crate::types::LabelConverter;
use crate::types::Type;
use crate::types::build_type::maybe_get_non_configurable_reason;
use blaze_starlark::label::LabelValue;

// Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAttrModuleApi.java:82 StarlarkAttrModuleApi (argument names; the others are the Rust parameter names).
const ALLOW_FILES_ARG: &str = "allow_files";
const DEFAULT_ARG: &str = "default";
const FLAGS_ARG: &str = "flags";
const PROVIDERS_ARG: &str = "providers";
const ALLOW_SINGLE_FILE_ARG: &str = "allow_single_file";
const VALUES_ARG: &str = "values";

// -------------------------------------------------------------------------------------------
// Descriptor

/// A descriptor of an attribute defined in Starlark (Java `StarlarkAttrModule.Descriptor`).
///
/// PORT: Java holds an `ImmutableAttributeFactory`; Blaze holds the `Attribute.Builder` it is
/// built from (the factory is `builder.build_partial()`), plus the Starlark values its
/// callable handles refer to (see `value_cell`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1192 StarlarkAttrModule.Descriptor
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct DescriptorGen<V: ValueLifetimeless> {
    /// Values referred to by handles in `builder` (empty once frozen).
    pending: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    handles: Vec<crate::rule_class::StarlarkCallable>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<DescriptorData>,
}

/// The immutable part of a descriptor.
#[derive(Debug)]
pub struct DescriptorData {
    /// The descriptor type, e.g. "label_list" (Java `Descriptor.name`).
    name: String,
    builder: AttributeBuilder,
}

starlark_complex_value!(pub Descriptor);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<DescriptorGen<Value<'v>>> for DescriptorGen<FrozenValue> {}

impl<'v> Freeze for Descriptor<'v> {
    type Frozen = FrozenDescriptor;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenDescriptor> {
        for (value, handle) in self.pending.into_iter().zip(self.handles) {
            PendingValue { value, handle }.freeze(freezer)?;
        }
        Ok(DescriptorGen {
            pending: Vec::new(),
            handles: Vec::new(),
            data: self.data,
        })
    }
}

impl<'v> Descriptor<'v> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1196 StarlarkAttrModule.Descriptor.<init>
    fn new(name: &str, builder: AttributeBuilder, pending: Vec<PendingValue<'v>>) -> Self {
        DescriptorGen {
            handles: pending.iter().map(|p| p.handle).collect(),
            pending: pending.iter().map(|p| p.value).collect(),
            data: Arc::new(DescriptorData {
                name: name.to_owned(),
                builder,
            }),
        }
    }
}

impl<'v, V: ValueLike<'v>> DescriptorGen<V> {
    fn factory(&self) -> ImmutableAttributeFactory {
        self.data.builder.build_partial()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1201 StarlarkAttrModule.Descriptor.hasDefault
    pub fn has_default(&self) -> bool {
        self.factory().is_value_set()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1205 StarlarkAttrModule.Descriptor.getValueSource
    pub fn get_value_source(&self) -> AttributeValueSource {
        self.data.builder.get_value_source()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1209 StarlarkAttrModule.Descriptor.getType
    pub fn get_type(&self) -> Type {
        self.factory().get_type()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1213 StarlarkAttrModule.Descriptor.build
    pub fn build(&self, name: &str) -> Attribute {
        self.factory().build(name)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1240 StarlarkAttrModule.Descriptor.getTransitionFactory
    pub fn get_transition_factory(&self) -> TransitionFactory {
        self.factory().get_transition_factory().clone()
    }

    /// The values (with their handles) this descriptor's attribute refers to; a rule or macro
    /// that builds the attribute keeps them so that their cells are filled when it freezes.
    pub fn pending_values(&self) -> Vec<PendingValue<'v>> {
        self.pending
            .iter()
            .zip(&self.handles)
            .map(|(v, h)| PendingValue {
                value: v.to_value(),
                handle: *h,
            })
            .collect()
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for DescriptorGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1218 StarlarkAttrModule.Descriptor.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<attr.{}>", self.data.name)
    }
}

#[starlark_value(type = "Attribute")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for DescriptorGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Value equality semantics - same as for native Attribute.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1224 StarlarkAttrModule.Descriptor.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(other) = as_descriptor(other) else {
            return Ok(false);
        };
        if Arc::ptr_eq(&self.data, &other.data) {
            return Ok(true);
        }
        Ok(self.data.name == other.data.name && self.factory() == other.factory())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1236 StarlarkAttrModule.Descriptor.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        // PORT: Java also hashes the attribute factory; equal factories hash alike, so hashing
        // the name alone is consistent with equals.
        self.data.name.hash(hasher);
        Ok(())
    }
}

/// A view of a descriptor value, frozen or not.
pub struct DescriptorRef<'v> {
    data: Arc<DescriptorData>,
    pending: Vec<PendingValue<'v>>,
}

impl<'v> DescriptorRef<'v> {
    fn factory(&self) -> ImmutableAttributeFactory {
        self.data.builder.build_partial()
    }

    pub fn has_default(&self) -> bool {
        self.data.builder.build_partial().is_value_set()
    }

    pub fn get_value_source(&self) -> AttributeValueSource {
        self.data.builder.get_value_source()
    }

    pub fn get_type(&self) -> Type {
        self.data.builder.build_partial().get_type()
    }

    pub fn build(&self, name: &str) -> Attribute {
        self.data.builder.build_partial().build(name)
    }

    pub fn get_transition_factory(&self) -> TransitionFactory {
        self.data
            .builder
            .build_partial()
            .get_transition_factory()
            .clone()
    }

    pub fn pending_values(&self) -> &[PendingValue<'v>] {
        &self.pending
    }
}

/// Java `x instanceof Descriptor`, for unfrozen and frozen descriptors.
pub fn as_descriptor<'v>(v: Value<'v>) -> Option<DescriptorRef<'v>> {
    if let Some(d) = Descriptor::from_value(v) {
        return Some(DescriptorRef {
            data: d.data.clone(),
            pending: d.pending_values(),
        });
    }
    if let Some(d) = v.downcast_ref::<FrozenDescriptor>() {
        return Some(DescriptorRef {
            data: d.data.clone(),
            pending: Vec::new(),
        });
    }
    None
}

// -------------------------------------------------------------------------------------------
// createAttribute

/// The named arguments of an `attr.*()` call (Java `optionMap`: absent or None values are
/// omitted). `configurable` and `for_dependency_resolution` keep Java's `UNBOUND` as `None`.
#[derive(Default)]
struct AttrArgs<'v> {
    configurable: Option<Value<'v>>,
    default: Option<Value<'v>>,
    materializer: Option<Value<'v>>,
    executable: Option<bool>,
    allow_files: Option<Value<'v>>,
    allow_single_file: Option<Value<'v>>,
    mandatory: Option<bool>,
    skip_validations: Option<bool>,
    providers: Option<Value<'v>>,
    for_dependency_resolution: Option<Value<'v>>,
    allow_rules: Option<Value<'v>>,
    cfg: Option<Value<'v>>,
    aspects: Option<Value<'v>>,
    flags: Option<Value<'v>>,
    allow_empty: Option<bool>,
    values: Option<Value<'v>>,
}

fn non_none<'v>(v: Option<Value<'v>>) -> Option<Value<'v>> {
    v.filter(|v| !v.is_none())
}

/// Throws if we're not in a Starlark evaluation context suitable for creating attribute
/// descriptors: .bzl initialization, or BUILD evaluation (for `analysis_test`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:108 StarlarkAttrModule.checkContext
fn check_context(eval: &Evaluator, what: &str) -> starlark::Result<()> {
    // PORT: Java tests the RuleDefinitionEnvironment thread-local of BUILD threads; in Blaze a
    // BUILD (or macro) evaluation carries the package builder payload. While a rule initializer
    // runs, the builder is off the thread (StarlarkRuleFunction.call) but the thread is still
    // the BUILD thread: it then carries the initializer's LabelConverter thread-local.
    if TargetDefinitionContext::from_or_null(eval).is_some()
        || blaze_starlark::ThreadContext::get(eval)
            .is_ok_and(|cx| cx.get_thread_local::<Arc<LabelConverter>>().is_some())
    {
        // BUILD initialization.
        return Ok(());
    }
    if BzlInitThreadContext::from_or_null(eval).is_none() {
        return Err(errorf(format!(
            "{what} can only be used during .bzl initialization (top-level evaluation) or package \
             evaluation (a BUILD file or macro)"
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:127 StarlarkAttrModule.setAllowedFileTypes
fn set_allowed_file_types(
    attr_name: &str,
    file_types_obj: Value,
    builder: &mut AttributeBuilder,
) -> starlark::Result<()> {
    if file_types_obj.unpack_bool() == Some(true) {
        builder.allowed_file_types(FileTypeSet::AnyFile);
    } else if file_types_obj.unpack_bool() == Some(false) {
        builder.allowed_file_types(FileTypeSet::NoFile);
    } else if sequence_elements(file_types_obj).is_some() {
        let arg = sequence_cast_strings(file_types_obj, "allow_files argument")?;
        let exts: Vec<&str> = arg.iter().map(String::as_str).collect();
        builder.allowed_file_types(FileTypeSet::of(vec![FileType::of(&exts)]));
    } else {
        return Err(errorf(format!(
            "{attr_name} should be a boolean or a string list"
        )));
    }
    Ok(())
}

/// Java `Starlark.type(x) == "function"` for a `def`/`lambda` (Java `StarlarkFunction`).
// PORT: starlark-rust does not distinguish `def` functions from builtins by type; both are
// "function".
pub fn is_starlark_function(v: Value) -> bool {
    v.get_type() == "function"
}

/// Java `StarlarkFunction.getParameterNames()` and `hasKwargs()`.
pub fn function_parameters(v: Value) -> (Vec<String>, bool) {
    use starlark::docs::DocItem;
    use starlark::docs::DocMember;
    match v.documentation() {
        DocItem::Member(DocMember::Function(f)) => {
            let p = &f.params;
            let mut names: Vec<String> = Vec::new();
            names.extend(p.pos_only.iter().map(|x| x.name.clone()));
            names.extend(p.pos_or_named.iter().map(|x| x.name.clone()));
            names.extend(p.named_only.iter().map(|x| x.name.clone()));
            names.extend(p.args.iter().map(|x| x.name.clone()));
            names.extend(p.kwargs.iter().map(|x| x.name.clone()));
            (names, p.kwargs.is_some())
        }
        _ => (Vec::new(), false),
    }
}

/// Converts an allowed value of `values = [...]` to an attribute value.
fn allowed_value(v: Value) -> starlark::Result<AttrValue> {
    if let Some(s) = v.unpack_str() {
        return Ok(AttrValue::String(s.to_owned()));
    }
    if let Some(i) = v.unpack_i32() {
        return Ok(AttrValue::Int(i));
    }
    if let Some(b) = v.unpack_bool() {
        return Ok(AttrValue::Bool(b));
    }
    Err(errorf(format!(
        "unsupported allowed value of type '{}'",
        starlark_type(v)
    )))
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:310 StarlarkAttrModule.createAttribute
fn create_attribute<'v>(
    type_: Type,
    doc: Option<&str>,
    arguments: &AttrArgs<'v>,
    eval: &mut Evaluator<'v, '_, '_>,
    name: &str,
) -> starlark::Result<(AttributeBuilder, Vec<PendingValue<'v>>)> {
    let mut pending = Vec::new();
    let mut builder = attr(name, type_);
    builder.starlark_defined();
    if let Some(doc) = doc {
        builder.set_doc(Some(trim_doc_string(doc)));
    }

    let default_value = non_none(arguments.default);
    let materializer = non_none(arguments.materializer);
    let is_mandatory = arguments.mandatory == Some(true);
    let configurable_param_set = non_none(arguments.configurable).is_some();

    if let Some(materializer) = materializer {
        if !is_starlark_function(materializer) {
            return Err(errorf(format!(
                "Expected a function in 'materializer' parameter, got '{}'",
                starlark_type(materializer)
            )));
        }
        // PORT: Java compares the raw `default` with the type's default value; a default given
        // as None counts as absent here as in Java's optionMap.
        let default_matches = match (type_.get_default_value(), default_value) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(_), None) => true,
            (Some(d), Some(v)) => type_
                .convert(v, None, None)
                .map(|c| c == d)
                .unwrap_or(false),
        };
        if !default_matches {
            return Err(errorf(
                "The 'materializer' and 'default' parameters are incompatible",
            ));
        }
        if is_mandatory {
            return Err(errorf(
                "The 'materializer' and 'mandatory' parameters are incompatible",
            ));
        }
        if configurable_param_set {
            return Err(errorf(
                "The 'materializer' and 'configurable' parameters are incompatible",
            ));
        }
        blaze_util::unported!("StarlarkAttrModule.StarlarkMaterializer");
    } else if let Some(default_value) = default_value {
        if is_starlark_function(default_value) {
            // Computed attribute. Non label type attributes already caused a type check error.
            // StarlarkComputedDefaultTemplate needs to know the names of all attributes that it
            // depends on. However, this method does not know anything about other attributes.
            // We solve this problem by asking the StarlarkCallbackHelper for the parameter names
            // used in the function definition, which must be the names of attributes used by the
            // callback.
            let callback = PendingValue::new(default_value, eval);
            let (parameter_names, _) = function_parameters(default_value);
            let semantics = blaze_starlark::ThreadContext::get(eval)?.semantics.clone();
            builder.value_starlark_computed_default(Arc::new(
                StarlarkComputedDefaultTemplate::new(type_, parameter_names, callback.handle)
                    .with_semantics(semantics),
            ));
            pending.push(callback);
        } else if let Some(late_bound) = late_bound_default_of(default_value) {
            builder.value_late_bound(late_bound);
        } else if default_value.get_type() == "NativeComputedDefault" {
            // TODO(b/200065655#comment3): This hack exists until default_copts and
            //  default_hdrs_check in package() is replaced by proper package defaults.
            let callable = PendingValue::new(default_value, eval);
            builder.value_native_computed_default(callable.handle);
            pending.push(callable);
        } else {
            let label_converter = label_converter_for_bzl(eval)?;
            builder
                .default_value(default_value, Some(&label_converter), Some(DEFAULT_ARG))
                .map_err(|e| errorf(e.to_string()))?;
        }
    }

    if let Some(flags_arg) = arguments.flags {
        for flag in sequence_noneable_cast_strings(flags_arg, FLAGS_ARG)? {
            builder.set_property_flag(&flag).map_err(eval_exception)?;
        }
    }

    if is_mandatory {
        builder
            .set_property_flag("MANDATORY")
            .map_err(eval_exception)?;
    }

    if let Some(for_dep_res) = arguments.for_dependency_resolution {
        builder
            .set_property_flag("FOR_DEPENDENCY_RESOLUTION_EXPLICITLY_SET")
            .map_err(eval_exception)?;
        if for_dep_res.unpack_bool() == Some(true) {
            builder
                .set_property_flag("FOR_DEPENDENCY_RESOLUTION")
                .map_err(eval_exception)?;
        } else {
            builder
                .remove_property_flag("FOR_DEPENDENCY_RESOLUTION")
                .map_err(eval_exception)?;
        }
    }

    if let Some(configurable) = non_none(arguments.configurable) {
        builder.configurable_attr_was_user_set();
        if configurable.unpack_bool() != Some(true) {
            // output, output_list, and license type attributes don't support the configurable=
            // arg, so no need to worry about calling nonconfigurable() twice.
            builder.nonconfigurable("This attribute was marked as nonconfigurable");
        }
    }

    if arguments.skip_validations == Some(true) {
        builder
            .set_property_flag("SKIP_VALIDATIONS")
            .map_err(eval_exception)?;
    }

    if arguments.allow_empty == Some(false) {
        builder
            .set_property_flag("NON_EMPTY")
            .map_err(eval_exception)?;
    }

    if arguments.executable == Some(true) {
        builder
            .set_property_flag("EXECUTABLE")
            .map_err(eval_exception)?;
        if non_none(arguments.cfg).is_none() {
            return Err(errorf(
                "cfg parameter is mandatory when executable=True is provided. Please see \
                 https://bazel.build/extending/rules#configurations for more details.",
            ));
        }
    }

    let allow_files = non_none(arguments.allow_files);
    let allow_single_file = non_none(arguments.allow_single_file);
    if allow_files.is_some() && allow_single_file.is_some() {
        return Err(errorf(
            "Cannot specify both allow_files and allow_single_file",
        ));
    }

    if let Some(file_types_obj) = allow_files {
        set_allowed_file_types(ALLOW_FILES_ARG, file_types_obj, &mut builder)?;
    } else if let Some(file_types_obj) = allow_single_file {
        set_allowed_file_types(ALLOW_SINGLE_FILE_ARG, file_types_obj, &mut builder)?;
        builder
            .set_property_flag("SINGLE_ARTIFACT")
            .map_err(eval_exception)?;
    } else if type_.get_label_class() == LabelClass::Dependency {
        builder.allowed_file_types(FileTypeSet::NoFile);
    }

    if let Some(rule_classes_obj) = non_none(arguments.allow_rules) {
        let classes = sequence_cast_strings(
            rule_classes_obj,
            "allowed rule classes for attribute definition",
        )?;
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        builder.allowed_rule_classes(&classes);
    }

    if let Some(values_arg) = arguments.values {
        let values = sequence_noneable_cast(values_arg, "Object", |_| true, VALUES_ARG)?;
        if !values.is_empty() {
            let values = values
                .into_iter()
                .map(allowed_value)
                .collect::<starlark::Result<Vec<_>>>()?;
            builder.allowed_values(Arc::new(AllowedValueSet::new(values)));
        }
    }

    if let Some(obj) = non_none(arguments.providers) {
        let providers_list = build_provider_predicate(
            &sequence_cast(obj, "Object", |_| true, PROVIDERS_ARG)?,
            PROVIDERS_ARG,
        )?;
        // If there is at least one empty set, there is no restriction.
        if providers_list.iter().all(|s| !s.is_empty()) {
            builder.mandatory_providers_list(providers_list);
        }
    }

    if let Some(trans) = non_none(arguments.cfg) {
        let converted = convert_cfg(eval, trans)?;
        let transition_factory = converted.factory;

        // Check whether something is attempting an invalid late bound transition.
        let is_split = transition_factory.is_split();
        if is_split && default_value.is_some_and(|d| late_bound_default_of(d).is_some()) {
            return Err(errorf(
                "late-bound attributes must not have a split configuration transition",
            ));
        }
        if is_split && materializer.is_some() {
            return Err(errorf(
                "materializing attributes must not have a split configuration transition",
            ));
        }

        // Check if this transition includes an analysis test or a Starlark transition.
        if converted.is_starlark_defined {
            if converted.is_for_analysis_testing {
                builder.has_analysis_test_transition();
            } else {
                builder.has_starlark_defined_transition();
            }
        }

        builder.cfg(transition_factory);
        pending.extend(converted.pending);
    }

    if let Some(obj) = non_none(arguments.aspects) {
        let aspects = sequence_cast(
            obj,
            "Aspect",
            |a| super::aspect::is_starlark_aspect(a),
            "aspects",
        )?;
        for aspect in aspects {
            let (info, aspect_pending) = aspect_info_of(aspect, eval)?;
            builder.aspect_info(&info).map_err(eval_exception)?;
            pending.extend(aspect_pending);
        }
    }

    Ok((builder, pending))
}

/// Builds a list of sets of accepted providers from Starlark list `obj`. The list can either be
/// a list of providers (in that case the result is a list with one set) or a list of lists of
/// providers (then the result is the list of sets).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:575 StarlarkAttrModule.buildProviderPredicate
pub fn build_provider_predicate(
    obj: &[Value],
    argument_name: &str,
) -> starlark::Result<Vec<Vec<StarlarkProviderIdentifier>>> {
    if obj.is_empty() {
        return Ok(Vec::new());
    }
    let is_list_of_providers = obj.iter().all(|o| provider_view(*o).is_some());
    if is_list_of_providers {
        Ok(vec![get_starlark_provider_identifiers(obj, argument_name)?])
    } else {
        // Java: `Sequence.cast(obj, Sequence.class, argumentName)` checks every element of the
        // outer list before getProvidersList looks into any sublist.
        let mut list_of_lists = Vec::new();
        for (i, sublist) in obj.iter().enumerate() {
            let Some(elems) = sequence_elements(*sublist) else {
                return Err(errorf(format!(
                    "at index {i} of {argument_name}, got element of type {}, want sequence",
                    starlark_type(*sublist)
                )));
            };
            list_of_lists.push(elems);
        }
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:613 StarlarkAttrModule.getProvidersList
        let mut providers_list = Vec::new();
        for elems in &list_of_lists {
            providers_list.push(get_starlark_provider_identifiers(elems, argument_name)?);
        }
        Ok(providers_list)
    }
}

/// Converts Starlark identifiers of providers to their internal representations (a set:
/// duplicates removed, first occurrence kept).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:598 StarlarkAttrModule.getStarlarkProviderIdentifiers
pub fn get_starlark_provider_identifiers(
    list: &[Value],
    argument_name: &str,
) -> starlark::Result<Vec<StarlarkProviderIdentifier>> {
    let mut result: Vec<StarlarkProviderIdentifier> = Vec::new();
    for (i, constructor) in list.iter().enumerate() {
        let Some(view) = provider_view(*constructor) else {
            return Err(errorf(format!(
                "at index {i} of {argument_name}, got element of type {}, want Provider",
                starlark_type(*constructor)
            )));
        };
        let _ = i;
        if !view.exported {
            return Err(errorf(
                "Providers should be top-level values in extension files that define them.",
            ));
        }
        let id =
            StarlarkProviderIdentifier::for_key(view.key.expect("exported provider has a key"));
        if !result.contains(&id) {
            result.push(id);
        }
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:623 StarlarkAttrModule.createAttrDescriptor
fn create_attr_descriptor<'v>(
    name: &str,
    doc: Option<&str>,
    kwargs: &AttrArgs<'v>,
    type_: Type,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Descriptor<'v>> {
    // We use an empty name now so that we can set it later.
    let (builder, pending) = create_attribute(type_, doc, kwargs, eval, "")?;
    Ok(Descriptor::new(name, builder, pending))
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:637 StarlarkAttrModule.createNonconfigurableAttrDescriptor
fn create_nonconfigurable_attr_descriptor<'v>(
    name: &str,
    doc: Option<&str>,
    kwargs: &AttrArgs<'v>,
    type_: Type,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Descriptor<'v>> {
    let why_not_configurable_reason =
        maybe_get_non_configurable_reason(type_).unwrap_or_else(|| panic!("{type_:?}"));
    // We use an empty name now so that we can set it later.
    let (mut builder, pending) = create_attribute(type_, doc, kwargs, eval, "")?;
    builder.nonconfigurable(why_not_configurable_reason);
    Ok(Descriptor::new(name, builder, pending))
}

/// The `doc` parameter: string or None.
fn doc_param<'v>(func: &str, doc: Option<Value<'v>>) -> starlark::Result<Option<&'v str>> {
    match doc {
        None => Ok(None),
        Some(d) if d.is_none() => Ok(None),
        Some(d) => match d.unpack_str() {
            Some(s) => Ok(Some(s)),
            None => Err(super::util::param_type_error(
                func,
                "doc",
                d,
                "string or NoneType",
            )),
        },
    }
}

/// `allow_files`: `@ParamType`s Boolean, Sequence<String>, NoneType.
fn allow_files_param<'v>(func: &str, v: Option<Value<'v>>) -> starlark::Result<Option<Value<'v>>> {
    match v {
        Some(f) if f.unpack_bool().is_none() && !f.is_none() && sequence_elements(f).is_none() => {
            Err(super::util::param_type_error(
                func,
                "allow_files",
                f,
                "bool, sequence, or NoneType",
            ))
        }
        other => Ok(other),
    }
}

/// `configurable`: bool or unbound.
fn configurable_param<'v>(func: &str, v: Option<Value<'v>>) -> starlark::Result<Option<Value<'v>>> {
    match v {
        Some(c) if c.unpack_bool().is_none() => Err(super::util::param_type_error(
            func,
            "configurable",
            c,
            "bool or unbound",
        )),
        other => Ok(other),
    }
}

/// The interpreter's check of the `default` argument against the classes its `@Param` allows
/// (`allowedTypes`, or the Java parameter type): `accepts` is the `isInstance` test of those
/// classes and `want` is `ParamDescriptor.getTypeErrorMessage()` (the classes' Starlark names
/// joined as "a", "a or b" or "a, b, or c", without the undocumented `NativeComputedDefault`).
/// An absent argument takes the parameter's default value, which is not checked.
// Java: net/starlark/java/eval/BuiltinFunction.java:498 BuiltinFunction.checkParamValue
// Java: net/starlark/java/eval/ParamDescriptor.java:123 ParamDescriptor.getTypeErrorMessage
fn check_default_param<'v>(
    func: &str,
    default: Option<Value<'v>>,
    accepts: impl Fn(Value<'v>) -> bool,
    want: &str,
) -> starlark::Result<()> {
    match default {
        Some(d) if !accepts(d) => Err(super::util::param_type_error(func, DEFAULT_ARG, d, want)),
        _ => Ok(()),
    }
}

/// Java `x instanceof Sequence`: a list, a tuple or a range
/// (net/starlark/java/eval/{StarlarkList,Tuple,RangeList}.java).
fn is_java_sequence(v: Value) -> bool {
    sequence_elements(v).is_some() || v.get_type() == "range"
}

/// Java `x instanceof Dict`.
fn is_java_dict(v: Value) -> bool {
    DictRef::from_value(v).is_some()
}

/// Java `x instanceof StarlarkInt`.
fn is_java_starlark_int(v: Value) -> bool {
    v.get_type() == "int"
}

/// Java `x instanceof Label`.
fn is_java_label(v: Value) -> bool {
    v.downcast_ref::<LabelValue>().is_some()
}

/// Java `x instanceof NativeComputedDefaultApi` (undocumented; never named in error messages).
fn is_native_computed_default(v: Value) -> bool {
    v.get_type() == "NativeComputedDefault"
}

// -------------------------------------------------------------------------------------------
// The `attr` module

/// The `attr` module object (Java `StarlarkAttrModule`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:91 StarlarkAttrModule
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkAttrModule;

starlark_simple_value!(StarlarkAttrModule);

impl fmt::Display for StarlarkAttrModule {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:659 StarlarkAttrModule.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<attr>")
    }
}

#[starlark_value(type = "attr")]
impl<'v> StarlarkValue<'v> for StarlarkAttrModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("attr", attr_methods);
        Some(RES.methods())
    }
}

/// The `attr` module when `--incompatible_no_attr_license` is false: Java's
/// `@StarlarkMethod(disableWithFlag = INCOMPATIBLE_NO_ATTR_LICENSE)` hides `license` (also from
/// `hasattr` and `dir`) when the flag is set, so the flag selects one of two method tables.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1179 StarlarkAttrModule.licenseAttribute
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkAttrModuleWithLicense;

starlark_simple_value!(StarlarkAttrModuleWithLicense);

impl fmt::Display for StarlarkAttrModuleWithLicense {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:659 StarlarkAttrModule.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<attr>")
    }
}

#[starlark_value(type = "attr")]
impl<'v> StarlarkValue<'v> for StarlarkAttrModuleWithLicense {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("attr", attr_methods_with_license);
        Some(RES.methods())
    }
}

fn attr_methods_with_license(builder: &mut MethodsBuilder) {
    attr_methods(builder);
    attr_license_method(builder);
}

#[starlark_module]
fn attr_license_method(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1179 StarlarkAttrModule.licenseAttribute
    fn license<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_context(eval, "attr.license()")?;
        let doc = doc_param("license", doc)?;
        let args = AttrArgs {
            default,
            mandatory: Some(mandatory),
            ..Default::default()
        };
        create_nonconfigurable_attr_descriptor("license", doc, &args, Type::License, eval)
    }
}

/// The `attr` object for `semantics` (see [`StarlarkAttrModuleWithLicense`]).
pub fn attr_for_semantics(
    semantics: &blaze_starlark::StarlarkSemantics,
) -> starlark::values::OwnedFrozenValue {
    let heap = starlark::values::FrozenHeap::new();
    let v = if semantics.incompatible_no_attr_license {
        heap.alloc(StarlarkAttrModule)
    } else {
        heap.alloc(StarlarkAttrModuleWithLicense)
    };
    // SAFETY: `v` was allocated on `heap`, which the OwnedFrozenValue keeps alive.
    unsafe { starlark::values::OwnedFrozenValue::new(heap.into_ref(), v) }
}

#[starlark_module]
fn attr_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:665 StarlarkAttrModule.intAttribute
    fn int<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named)] values: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: `StarlarkInt defaultValue`.
        check_default_param("int", default, is_java_starlark_int, "int")?;
        // TODO(bazel-team): Replace literal strings with constants.
        check_context(eval, "attr.int()")?;
        let doc = doc_param("int", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("int", configurable)?,
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(0))),
            mandatory: Some(mandatory),
            values: Some(values.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            ..Default::default()
        };
        create_attr_descriptor("int", doc, &args, Type::Integer, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:692 StarlarkAttrModule.stringAttribute
    fn string<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named)] values: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_context(eval, "attr.string()")?;
        if let Some(d) = default
            && d.unpack_str().is_none()
            && d.get_type() != "NativeComputedDefault"
        {
            // Java: @ParamType(String), @ParamType(NativeComputedDefaultApi) (undocumented).
            return Err(super::util::param_type_error(
                "string", "default", d, "string",
            ));
        }
        let doc = doc_param("string", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("string", configurable)?,
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(""))),
            mandatory: Some(mandatory),
            values: Some(values.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            ..Default::default()
        };
        create_attr_descriptor("string", doc, &args, Type::String, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:718 StarlarkAttrModule.labelAttribute
    fn label<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] materializer: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] executable: bool,
        #[starlark(require = named)] allow_files: Option<Value<'v>>,
        #[starlark(require = named)] allow_single_file: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named, default = false)] skip_validations: bool,
        #[starlark(require = named)] providers: Option<Value<'v>>,
        #[starlark(require = named)] for_dependency_resolution: Option<Value<'v>>,
        #[starlark(require = named)] allow_rules: Option<Value<'v>>,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] aspects: Option<Value<'v>>,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Label, String, LateBoundDefaultApi, NativeComputedDefaultApi,
        // StarlarkFunction, NoneType.
        check_default_param(
            "label",
            default,
            |d| {
                is_java_label(d)
                    || d.unpack_str().is_some()
                    || late_bound_default_of(d).is_some()
                    || is_native_computed_default(d)
                    || is_starlark_function(d)
                    || d.is_none()
            },
            "Label, string, LateBoundDefault, function, or NoneType",
        )?;
        check_context(eval, "attr.label()")?;
        check_materializer_flag(eval, materializer)?;
        let doc = doc_param("label", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("label", configurable)?,
            default,
            materializer,
            executable: Some(executable),
            allow_files: allow_files_param("label", allow_files)?,
            allow_single_file,
            mandatory: Some(mandatory),
            skip_validations: Some(skip_validations),
            providers,
            for_dependency_resolution,
            allow_rules,
            cfg,
            aspects,
            flags,
            ..Default::default()
        };
        let (builder, pending) = create_attribute(Type::Label, doc, &args, eval, "label")?;
        Ok(Descriptor::new("label", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:777 StarlarkAttrModule.stringListAttribute
    fn string_list<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = false)] mandatory: bool,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Sequence<String>, NativeComputedDefaultApi.
        check_default_param(
            "string_list",
            default,
            |d| is_java_sequence(d) || is_native_computed_default(d),
            "sequence",
        )?;
        check_context(eval, "attr.string_list()")?;
        let doc = doc_param("string_list", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("string_list", configurable)?,
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        create_attr_descriptor("string_list", doc, &args, Type::StringList, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:803 StarlarkAttrModule.intListAttribute
    fn int_list<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = false)] mandatory: bool,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Sequence<StarlarkInt>.
        check_default_param("int_list", default, is_java_sequence, "sequence")?;
        check_context(eval, "attr.int_list()")?;
        let doc = doc_param("int_list", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("int_list", configurable)?,
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        create_attr_descriptor("int_list", doc, &args, Type::IntegerList, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:829 StarlarkAttrModule.labelListAttribute
    fn label_list<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] materializer: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named)] allow_files: Option<Value<'v>>,
        #[starlark(require = named)] allow_rules: Option<Value<'v>>,
        #[starlark(require = named)] providers: Option<Value<'v>>,
        #[starlark(require = named)] for_dependency_resolution: Option<Value<'v>>,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named, default = false)] skip_validations: bool,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] aspects: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Sequence<Label>, StarlarkFunction.
        check_default_param(
            "label_list",
            default,
            |d| is_java_sequence(d) || is_starlark_function(d),
            "sequence or function",
        )?;
        check_context(eval, "attr.label_list()")?;
        check_materializer_flag(eval, materializer)?;
        let doc = doc_param("label_list", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("label_list", configurable)?,
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            materializer,
            allow_files: allow_files_param("label_list", allow_files)?,
            allow_rules,
            providers,
            for_dependency_resolution,
            flags,
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            cfg,
            aspects,
            skip_validations: Some(skip_validations),
            ..Default::default()
        };
        let (builder, pending) = create_attribute(Type::LabelList, doc, &args, eval, "label_list")?;
        Ok(Descriptor::new("label_list", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:886 StarlarkAttrModule.dormantLabelAttribute
    fn dormant_label<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_dormant_deps_flag(eval, "dormant_label")?;
        // Java: @ParamType Label, String, NoneType.
        check_default_param(
            "dormant_label",
            default,
            |d| is_java_label(d) || d.unpack_str().is_some() || d.is_none(),
            "Label, string, or NoneType",
        )?;
        check_context(eval, "attr.dormant_label()")?;
        let doc = doc_param("dormant_label", doc)?;
        let args = AttrArgs {
            default,
            mandatory: Some(mandatory),
            ..Default::default()
        };
        let (builder, pending) =
            create_attribute(Type::DormantLabel, doc, &args, eval, "dormant_label")?;
        Ok(Descriptor::new("dormant_label", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:902 StarlarkAttrModule.dormantLabelListAttribute
    fn dormant_label_list<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_dormant_deps_flag(eval, "dormant_label_list")?;
        // Java: @ParamType Sequence<Label>.
        check_default_param("dormant_label_list", default, is_java_sequence, "sequence")?;
        check_context(eval, "attr.dormant_label_list()")?;
        let doc = doc_param("dormant_label_list", doc)?;
        let args = AttrArgs {
            default: Some(default.unwrap_or_else(|| eval.heap().alloc(Vec::<Value>::new()))),
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        let (builder, pending) = create_attribute(
            Type::DormantLabelList,
            doc,
            &args,
            eval,
            "dormant_label_list",
        )?;
        Ok(Descriptor::new("dormant_label_list", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:919 StarlarkAttrModule.stringKeyedLabelDictAttribute
    fn string_keyed_label_dict<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named)] allow_files: Option<Value<'v>>,
        #[starlark(require = named)] allow_rules: Option<Value<'v>>,
        #[starlark(require = named)] providers: Option<Value<'v>>,
        #[starlark(require = named)] for_dependency_resolution: Option<Value<'v>>,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] aspects: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Dict, StarlarkFunction.
        check_default_param(
            "string_keyed_label_dict",
            default,
            |d| is_java_dict(d) || is_starlark_function(d),
            "dict or function",
        )?;
        check_context(eval, "attr.string_keyed_label_dict()")?;
        let doc = doc_param("string_keyed_label_dict", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("string_keyed_label_dict", configurable)?,
            default: Some(default.unwrap_or_else(|| empty_dict(eval))),
            allow_files: allow_files_param("string_keyed_label_dict", allow_files)?,
            allow_rules,
            providers,
            for_dependency_resolution,
            flags,
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            cfg,
            aspects,
            ..Default::default()
        };
        let (builder, pending) = create_attribute(
            Type::LabelDictUnary,
            doc,
            &args,
            eval,
            "string_keyed_label_dict",
        )?;
        Ok(Descriptor::new("string_keyed_label_dict", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:970 StarlarkAttrModule.labelKeyedStringDictAttribute
    fn label_keyed_string_dict<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named)] allow_files: Option<Value<'v>>,
        #[starlark(require = named)] allow_rules: Option<Value<'v>>,
        #[starlark(require = named)] providers: Option<Value<'v>>,
        #[starlark(require = named)] for_dependency_resolution: Option<Value<'v>>,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named, default = false)] skip_validations: bool,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] aspects: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: @ParamType Dict, StarlarkFunction.
        check_default_param(
            "label_keyed_string_dict",
            default,
            |d| is_java_dict(d) || is_starlark_function(d),
            "dict or function",
        )?;
        check_context(eval, "attr.label_keyed_string_dict()")?;
        let doc = doc_param("label_keyed_string_dict", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("label_keyed_string_dict", configurable)?,
            default: Some(default.unwrap_or_else(|| empty_dict(eval))),
            allow_files: allow_files_param("label_keyed_string_dict", allow_files)?,
            allow_rules,
            providers,
            for_dependency_resolution,
            flags,
            mandatory: Some(mandatory),
            skip_validations: Some(skip_validations),
            allow_empty: Some(allow_empty),
            cfg,
            aspects,
            ..Default::default()
        };
        let (builder, pending) = create_attribute(
            Type::LabelKeyedStringDict,
            doc,
            &args,
            eval,
            "label_keyed_string_dict",
        )?;
        Ok(Descriptor::new("label_keyed_string_dict", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1024 StarlarkAttrModule.labelListDictAttribute
    fn label_list_dict<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named)] allow_files: Option<Value<'v>>,
        #[starlark(require = named)] allow_rules: Option<Value<'v>>,
        #[starlark(require = named)] providers: Option<Value<'v>>,
        #[starlark(require = named)] for_dependency_resolution: Option<Value<'v>>,
        #[starlark(require = named)] flags: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        #[starlark(require = named, default = false)] skip_validations: bool,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] aspects: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: `Dict<?, ?> defaultValue`.
        check_default_param("label_list_dict", default, is_java_dict, "dict")?;
        check_context(eval, "attr.label_list_dict()")?;
        let doc = doc_param("label_list_dict", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("label_list_dict", configurable)?,
            default: Some(default.unwrap_or_else(|| empty_dict(eval))),
            allow_files: allow_files_param("label_list_dict", allow_files)?,
            allow_rules,
            providers,
            for_dependency_resolution,
            flags,
            mandatory: Some(mandatory),
            skip_validations: Some(skip_validations),
            allow_empty: Some(allow_empty),
            cfg,
            aspects,
            ..Default::default()
        };
        let (builder, pending) =
            create_attribute(Type::LabelListDict, doc, &args, eval, "label_list_dict")?;
        Ok(Descriptor::new("label_list_dict", builder, pending))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1078 StarlarkAttrModule.boolAttribute
    fn bool<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: `Boolean defaultValue` (defaultValue = "False").
        check_default_param("bool", default, |d| d.unpack_bool().is_some(), "bool")?;
        check_context(eval, "attr.bool()")?;
        let doc = doc_param("bool", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("bool", configurable)?,
            default: Some(default.unwrap_or_else(|| Value::new_bool(false))),
            mandatory: Some(mandatory),
            ..Default::default()
        };
        create_attr_descriptor("bool", doc, &args, Type::Boolean, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1096 StarlarkAttrModule.outputAttribute
    fn output<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_context(eval, "attr.output()")?;
        let doc = doc_param("output", doc)?;
        let args = AttrArgs {
            mandatory: Some(mandatory),
            ..Default::default()
        };
        create_nonconfigurable_attr_descriptor("output", doc, &args, Type::Output, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1109 StarlarkAttrModule.outputListAttribute
    fn output_list<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        check_context(eval, "attr.output_list()")?;
        let doc = doc_param("output_list", doc)?;
        // The resulting Attribute does not have the nonconfigurable bit set, but is still
        // nonconfigurable in practice because Attribute#isConfigurable specifically checks
        // whether the attribute has LabelClass.OUTPUT.
        let args = AttrArgs {
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        create_attr_descriptor("output_list", doc, &args, Type::OutputList, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1127 StarlarkAttrModule.stringDictAttribute
    fn string_dict<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: `Dict<?, ?> defaultValue`.
        check_default_param("string_dict", default, is_java_dict, "dict")?;
        check_context(eval, "attr.string_dict()")?;
        let doc = doc_param("string_dict", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("string_dict", configurable)?,
            default: Some(default.unwrap_or_else(|| empty_dict(eval))),
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        create_attr_descriptor("string_dict", doc, &args, Type::StringDict, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:1153 StarlarkAttrModule.stringListDictAttribute
    fn string_list_dict<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(default = true)] allow_empty: bool,
        #[starlark(require = named)] configurable: Option<Value<'v>>,
        #[starlark(require = named)] default: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] mandatory: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Descriptor<'v>> {
        // Java: `Dict<?, ?> defaultValue`.
        check_default_param("string_list_dict", default, is_java_dict, "dict")?;
        check_context(eval, "attr.string_list_dict()")?;
        let doc = doc_param("string_list_dict", doc)?;
        let args = AttrArgs {
            configurable: configurable_param("string_list_dict", configurable)?,
            default: Some(default.unwrap_or_else(|| empty_dict(eval))),
            mandatory: Some(mandatory),
            allow_empty: Some(allow_empty),
            ..Default::default()
        };
        create_attr_descriptor("string_list_dict", doc, &args, Type::StringListDict, eval)
    }
}

fn empty_dict<'v>(eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
    eval.heap()
        .alloc(starlark::values::dict::AllocDict(std::iter::empty::<(
            Value,
            Value,
        )>()))
}

/// Java `@Param(enableOnlyWithFlag = EXPERIMENTAL_DORMANT_DEPS)` on `materializer`.
fn check_materializer_flag(eval: &Evaluator, materializer: Option<Value>) -> starlark::Result<()> {
    if materializer.is_some()
        && !blaze_starlark::ThreadContext::get(eval)?
            .semantics
            .experimental_dormant_deps
    {
        return Err(errorf(
            "parameter 'materializer' is experimental and thus unavailable with the current \
             flags. It may be enabled by setting --experimental_dormant_deps",
        ));
    }
    Ok(())
}

/// Java `@StarlarkMethod(enableOnlyWithFlag = EXPERIMENTAL_DORMANT_DEPS)`.
fn check_dormant_deps_flag(eval: &Evaluator, name: &str) -> starlark::Result<()> {
    if !blaze_starlark::ThreadContext::get(eval)?
        .semantics
        .experimental_dormant_deps
    {
        return Err(errorf(format!(
            "'attr' value has no field or method '{name}'"
        )));
    }
    Ok(())
}

/// Registers `attr` (Java: StarlarkGlobalsImpl.getFixedBzlToplevels).
#[starlark_module]
pub fn register_attr(builder: &mut GlobalsBuilder) {
    const attr: StarlarkAttrModule = StarlarkAttrModule;
}
