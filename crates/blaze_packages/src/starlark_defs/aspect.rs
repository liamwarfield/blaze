//! Port of the `aspect()` builtin (`StarlarkRuleClassFunctions.aspect`) and the loading-phase
//! parts of `packages/StarlarkDefinedAspect.java` and `AspectPropagationEdgesSupplier`.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
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
use starlark::values::starlark_value;

use super::attr_module::DescriptorRef;
use super::attr_module::build_provider_predicate;
use super::attr_module::get_starlark_provider_identifiers;
use super::attr_module::is_starlark_function;
use super::rule_class_functions::attr_object_to_attributes_list;
use super::rule_class_functions::parse_exec_groups;
use super::rule_class_functions::parse_labels;
use super::rule_class_functions::parse_toolchain_types;
use super::subrule;
use super::util::errorf;
use super::util::eval_exception;
use super::util::label_converter_for_bzl;
use super::util::sequence_cast;
use super::util::sequence_cast_strings;
use super::util::starlark_type;
use super::util::trim_doc_string;
use super::value_cell::PendingValue;
use crate::attribute::Attribute;
use crate::attribute::AttributeValueSource;
use crate::bzl_context::BzlInitThreadContext;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::StarlarkAspectInfo;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::types::AttrValue;
use crate::types::Type;

/// Java `AspectPropagationEdgesSupplier<String>` / `<Label>`: a fixed list or a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationEdges<T: std::hash::Hash + Eq> {
    /// Java `FixedListSupplier`.
    Fixed(IndexSet<T>),
    /// Java `FunctionSupplier`: the function is [`StarlarkDefinedAspectGen`]'s field.
    Function,
}

/// Java `AspectPropagationEdgesSupplier.ALL_TOOLCHAINS`: the pseudo-label standing for "all
/// toolchain types" in `toolchains_aspects`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:48 AspectPropagationEdgesSupplier.ALL_TOOLCHAINS
pub fn all_toolchains() -> Label {
    Label::parse_canonical_unchecked("//__toolchains_aspects__:all")
}

/// The non-Starlark part of an aspect.
#[derive(Debug)]
pub struct AspectData {
    pub documentation: Option<String>,
    pub attribute_aspects: PropagationEdges<String>,
    pub toolchains_aspects: PropagationEdges<Label>,
    pub attributes: Vec<Attribute>,
    pub required_providers: Vec<Vec<StarlarkProviderIdentifier>>,
    pub required_aspect_providers: Vec<Vec<StarlarkProviderIdentifier>>,
    pub provides: Vec<StarlarkProviderIdentifier>,
    /// Java `paramAttributes`.
    pub param_attributes: IndexSet<String>,
    pub fragments: IndexSet<String>,
    pub toolchain_types: IndexSet<ToolchainTypeRequirement>,
    pub apply_to_generating_rules: bool,
    pub exec_compatible_with: IndexSet<Label>,
    pub exec_groups: IndexMap<String, DeclaredExecGroup>,
    /// Identity of an unexported aspect (Java identity token).
    symbol: u64,
    /// Java `StarlarkAspectClass(extensionKey, exportedName)` once exported.
    exported: OnceLock<(Label, String)>,
}

impl AspectData {
    /// Java `getName()` / `StarlarkAspectClass.getName()`: `<bzl label>%<exported name>`.
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:33 StarlarkAspectClass.<init>
    pub fn get_name(&self) -> Option<String> {
        self.exported
            .get()
            .map(|(label, name)| format!("{label}%{name}"))
    }

    pub fn is_exported(&self) -> bool {
        self.exported.get().is_some()
    }

    /// The `(extension label, exported name)` of an exported aspect.
    pub fn exported(&self) -> Option<&(Label, String)> {
        self.exported.get()
    }
}

/// A Starlark-defined aspect (Java `StarlarkDefinedAspect`).
// Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:48 StarlarkDefinedAspect
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkDefinedAspectGen<V: ValueLifetimeless> {
    implementation: V,
    attr_aspects_function: Option<V>,
    toolchains_aspects_function: Option<V>,
    propagation_predicate: Option<V>,
    required_aspects: Vec<V>,
    subrules: Vec<V>,
    /// Values referred to by handles in `data.attributes` (empty once frozen).
    pending: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    handles: Vec<crate::rule_class::StarlarkCallable>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<AspectData>,
}

starlark_complex_value!(pub StarlarkDefinedAspect);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkDefinedAspectGen<Value<'v>>>
    for StarlarkDefinedAspectGen<FrozenValue>
{
}

fn freeze_opt(v: Option<Value>, freezer: &Freezer) -> FreezeResult<Option<FrozenValue>> {
    match v {
        Some(v) => Ok(Some(v.freeze(freezer)?)),
        None => Ok(None),
    }
}

impl<'v> Freeze for StarlarkDefinedAspect<'v> {
    type Frozen = FrozenStarlarkDefinedAspect;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkDefinedAspect> {
        for (value, handle) in self.pending.into_iter().zip(self.handles) {
            PendingValue { value, handle }.freeze(freezer)?;
        }
        Ok(StarlarkDefinedAspectGen {
            implementation: self.implementation.freeze(freezer)?,
            attr_aspects_function: freeze_opt(self.attr_aspects_function, freezer)?,
            toolchains_aspects_function: freeze_opt(self.toolchains_aspects_function, freezer)?,
            propagation_predicate: freeze_opt(self.propagation_predicate, freezer)?,
            required_aspects: self.required_aspects.freeze(freezer)?,
            subrules: self.subrules.freeze(freezer)?,
            pending: Vec::new(),
            handles: Vec::new(),
            data: self.data,
        })
    }
}

impl<'v, V: ValueLike<'v>> StarlarkDefinedAspectGen<V> {
    pub fn data(&self) -> &Arc<AspectData> {
        &self.data
    }

    /// Java `getImplementation()`.
    pub fn get_implementation(&self) -> Value<'v> {
        self.implementation.to_value()
    }

    /// Java `getRequiredAspects()`.
    pub fn get_required_aspects(&self) -> Vec<Value<'v>> {
        self.required_aspects.iter().map(|v| v.to_value()).collect()
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkDefinedAspectGen<V> {
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:162 StarlarkDefinedAspect.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<aspect>")
    }
}

#[starlark_value(type = "Aspect")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkDefinedAspectGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:435 StarlarkDefinedAspect.equals
    // PORT: Java compares all fields and the identity token; the identity token is unique per
    // aspect() call (or the exported aspect class), so identity decides.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match aspect_data(other) {
            Some(o) => same_aspect(&self.data, &o),
            None => false,
        })
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.data.exported.get() {
            Some(e) => e.hash(hasher),
            None => self.data.symbol.hash(hasher),
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:184 StarlarkDefinedAspect.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if self.data.exported.get().is_some() {
            return Ok(());
        }
        let Some(ctx) = BzlInitThreadContext::from_or_null(eval) else {
            return Ok(());
        };
        let label = ctx.bzl_file.clone();
        drop(ctx);
        let _ = self.data.exported.set((label, variable_name.to_owned()));
        Ok(())
    }
}

fn same_aspect(a: &Arc<AspectData>, b: &Arc<AspectData>) -> bool {
    match (a.exported.get(), b.exported.get()) {
        (Some(x), Some(y)) => x == y,
        _ => Arc::ptr_eq(a, b),
    }
}

/// The shared data of an aspect value (frozen or not).
pub fn aspect_data(v: Value) -> Option<Arc<AspectData>> {
    if let Some(a) = StarlarkDefinedAspect::from_value(v) {
        return Some(a.data.clone());
    }
    v.downcast_ref::<FrozenStarlarkDefinedAspect>()
        .map(|a| a.data.clone())
}

/// Java `x instanceof StarlarkAspect`.
// PORT: native aspects exposed to Starlark (StarlarkNativeAspect) are not created by any ported
// builtin.
pub fn is_starlark_aspect(v: Value) -> bool {
    aspect_data(v).is_some()
}

fn required_aspects_of<'v>(v: Value<'v>) -> Vec<Value<'v>> {
    if let Some(a) = StarlarkDefinedAspect::from_value(v) {
        return a.get_required_aspects();
    }
    match v.downcast_ref::<FrozenStarlarkDefinedAspect>() {
        Some(a) => a.required_aspects.iter().map(|v| v.to_value()).collect(),
        None => Vec::new(),
    }
}

/// What `AspectsList` needs to know about aspect value `v` (see A1's [`StarlarkAspectInfo`]), and
/// the values its handles refer to.
pub fn aspect_info_of<'v>(
    v: Value<'v>,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<(StarlarkAspectInfo, Vec<PendingValue<'v>>)> {
    let Some(data) = aspect_data(v) else {
        return Err(errorf(format!(
            "got value of type '{}', want 'Aspect'",
            starlark_type(v)
        )));
    };
    let handle = PendingValue::new(v, eval);
    let mut pending = vec![handle];
    let mut required_aspects = Vec::new();
    for required in required_aspects_of(v) {
        let (info, p) = aspect_info_of(required, eval)?;
        required_aspects.push(info);
        pending.extend(p);
    }
    Ok((
        StarlarkAspectInfo {
            aspect: handle.handle,
            name: data.get_name().unwrap_or_default(),
            exported: data.is_exported(),
            required_aspects,
            required_parameters: data.param_attributes.iter().cloned().collect(),
            aspect_attributes: data.attributes.clone(),
        },
        pending,
    ))
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:194 AspectPropagationEdgesSupplier.parseAttrAspects
fn parse_attr_aspects(
    raw_attr_aspects: Value,
    allow_all: bool,
) -> starlark::Result<IndexSet<String>> {
    let attr_aspects = sequence_cast_strings(raw_attr_aspects, "attr_aspects")?;
    let mut builder = IndexSet::new();
    for attr_name in &attr_aspects {
        if attr_name == "*" {
            if !allow_all {
                return Err(errorf("'*' is not allowed in 'attr_aspects' list"));
            } else if attr_aspects.len() != 1 {
                return Err(errorf("'*' must be the only string in 'attr_aspects' list"));
            }
        }
        if !attr_name.starts_with('_') {
            builder.insert(attr_name.clone());
        } else {
            // Implicit attribute names mean either implicit or late-bound attributes
            // (``$attr`` or ``:attr``). Depend on both.
            builder.insert(
                AttributeValueSource::ComputedDefault
                    .convert_to_native_name(attr_name)
                    .map_err(eval_exception)?,
            );
            builder.insert(
                AttributeValueSource::LateBound
                    .convert_to_native_name(attr_name)
                    .map_err(eval_exception)?,
            );
        }
    }
    Ok(builder)
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:221 AspectPropagationEdgesSupplier.parseToolchainsAspects
fn parse_toolchains_aspects(
    raw_toolchains_aspects: Value,
    eval: &Evaluator,
) -> starlark::Result<IndexSet<Label>> {
    let toolchains_aspects = sequence_cast_strings(raw_toolchains_aspects, "toolchains_aspects")?;
    if toolchains_aspects.len() == 1 && toolchains_aspects[0] == "*" {
        return Ok(IndexSet::from([all_toolchains()]));
    }
    let label_converter = label_converter_for_bzl(eval)?;
    let mut parsed_labels = IndexSet::new();
    for input in &toolchains_aspects {
        if input == "*" {
            // This is already handled if the list has a single '*' item in it
            return Err(errorf(
                "'*' must be the only item in 'toolchains_aspects' list",
            ));
        }
        match label_converter.convert(input) {
            Ok(label) => {
                parsed_labels.insert(label);
            }
            Err(e) => {
                return Err(errorf(format!(
                    "Unable to parse label '{input}' in attribute 'toolchains_aspects': {}",
                    e.get_message()
                )));
            }
        }
    }
    Ok(parsed_labels)
}

/// Java `Attribute.getDefaultValue(null)` compared with Java `Objects.equals`.
fn default_is(attribute: &Attribute, v: &AttrValue) -> bool {
    attribute.get_default_value(None).as_ref() == Some(v)
}

#[starlark_module]
pub fn register_aspect(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1323 StarlarkRuleClassFunctions.aspect
    fn aspect<'v>(
        implementation: Value<'v>,
        attr_aspects: Option<Value<'v>>,
        toolchains_aspects: Option<Value<'v>>,
        attrs: Option<Value<'v>>,
        required_providers: Option<Value<'v>>,
        required_aspect_providers: Option<Value<'v>>,
        provides: Option<Value<'v>>,
        requires: Option<Value<'v>>,
        propagation_predicate: Option<Value<'v>>,
        fragments: Option<Value<'v>>,
        host_fragments: Option<Value<'v>>,
        toolchains: Option<Value<'v>>,
        doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] apply_to_generating_rules: bool,
        #[starlark(require = named)] exec_compatible_with: Option<Value<'v>>,
        #[starlark(require = named)] exec_groups: Option<Value<'v>>,
        #[starlark(require = named)] subrules: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkDefinedAspect<'v>> {
        // Java: the interpreter checks `@Param(name = "propagation_predicate", allowedTypes =
        // {StarlarkFunction, NoneType})` before the method body
        // (lib/starlarkbuildapi/StarlarkRuleFunctionsApi.java:913; text:
        // net/starlark/java/eval/BuiltinFunction.java:498 BuiltinFunction.checkParamValue).
        if let Some(raw) = propagation_predicate
            && !raw.is_none()
            && !is_starlark_function(raw)
        {
            return Err(errorf(format!(
                "in call to aspect(), parameter 'propagation_predicate' got value of type '{}', \
                 want 'function or NoneType'",
                starlark_type(raw)
            )));
        }
        let empty = || eval.heap().alloc(Vec::<Value>::new());
        let attr_aspects = attr_aspects.unwrap_or_else(empty);
        let toolchains_aspects = toolchains_aspects.unwrap_or_else(empty);
        let attrs = attrs.unwrap_or_else(|| {
            eval.heap()
                .alloc(starlark::values::dict::AllocDict(std::iter::empty::<(
                    Value,
                    Value,
                )>()))
        });
        let required_providers_arg = required_providers.unwrap_or_else(empty);
        let required_aspect_providers_arg = required_aspect_providers.unwrap_or_else(empty);
        let provides_arg = provides.unwrap_or_else(empty);
        let requires = requires.unwrap_or_else(empty);
        let fragments = fragments.unwrap_or_else(empty);
        let _host_fragments = host_fragments.unwrap_or_else(empty);
        let toolchains = toolchains.unwrap_or_else(empty);
        let raw_exec_compatible_with = exec_compatible_with.unwrap_or_else(empty);
        let subrules_unchecked = subrules.unwrap_or_else(empty);

        for (name, v) in [
            ("attr_aspects", attr_aspects),
            ("toolchains_aspects", toolchains_aspects),
        ] {
            if super::util::sequence_elements(v).is_none() && !is_starlark_function(v) {
                return Err(super::util::param_type_error(
                    "aspect",
                    name,
                    v,
                    "sequence or function",
                ));
            }
        }
        if !is_starlark_function(implementation) {
            return Err(super::util::param_type_error(
                "aspect",
                "implementation",
                implementation,
                "function",
            ));
        }
        let doc = match doc {
            Some(d) if !d.is_none() => match d.unpack_str() {
                Some(s) => Some(s),
                None => {
                    return Err(super::util::param_type_error(
                        "aspect",
                        "doc",
                        d,
                        "string or NoneType",
                    ));
                }
            },
            _ => None,
        };

        // Ensure we're initializing a .bzl file.
        BzlInitThreadContext::from_or_fail(eval, "aspect()").map_err(eval_exception)?;
        let label_converter = label_converter_for_bzl(eval)?;

        let mut descriptors: Vec<(String, DescriptorRef<'v>)> =
            attr_object_to_attributes_list(attrs)?;

        let subrule_values = sequence_cast(
            subrules_unchecked,
            "Subrule",
            |s| subrule::is_subrule(s),
            "subrules",
        )?;
        if !subrule_values.is_empty() {
            let semantics = blaze_starlark::ThreadContext::get(eval)?.semantics.clone();
            if !semantics.experimental_rule_extension_api {
                super::rule_class_functions::fail_if_called_outside_allowlist(
                    eval,
                    super::rule_class_functions::ALLOWLIST_RULE_EXTENSION_API,
                )?;
            }
        }
        let subrule_attributes = subrule::discover_attributes(&subrule_values)?;
        if !subrule_attributes.is_empty() {
            descriptors.extend(subrule_attributes);
        }

        let mut attributes: Vec<Attribute> = Vec::new();
        let mut required_params: IndexSet<String> = IndexSet::new();
        let mut pending: Vec<PendingValue<'v>> = Vec::new();
        for (native_name, descriptor) in &descriptors {
            let mut has_default = descriptor.has_default();
            let attribute = descriptor.build(native_name);
            pending.extend(descriptor.pending_values().iter().copied());

            // "configurable" may only be user-set for symbolic macros, not aspects.
            if attribute.configurable_attr_was_user_set() {
                return Err(errorf(format!(
                    "attribute '{native_name}' has the 'configurable' argument set, which is not \
                     allowed in aspect definitions"
                )));
            }

            if attribute.is_materializing() {
                return Err(errorf(format!(
                    "attribute '{native_name}' has a materializer, which is not allowed on aspects"
                )));
            }

            if attribute.get_type() == Type::DormantLabel
                || attribute.get_type() == Type::DormantLabelList
            {
                return Err(errorf(format!(
                    "attribute '{}' has a dormant label type, which is not allowed on aspects",
                    attribute.get_public_name()
                )));
            }

            if !Attribute::is_implicit_name(native_name)
                && !Attribute::is_analysis_dependent(native_name)
            {
                if attribute.get_type() == Type::String {
                    // isValueSet() is always true for attr.string as default value is "" by
                    // default.
                    has_default = !default_is(&attribute, &AttrValue::String(String::new()));
                } else if attribute.get_type() == Type::Integer {
                    // isValueSet() is always true for attr.int as default value is 0 by default.
                    has_default = !default_is(&attribute, &AttrValue::Int(0));
                } else if attribute.get_type() == Type::Boolean {
                    has_default = !default_is(&attribute, &AttrValue::Bool(false));
                } else {
                    return Err(errorf(format!(
                        "Aspect parameter attribute '{native_name}' must have type 'bool', 'int' \
                         or 'string'."
                    )));
                }

                if has_default && attribute.check_allowed_values() {
                    let allowed = attribute.get_allowed_values().unwrap();
                    let default_val = attribute.get_default_value(None);
                    if let Some(default_val) = &default_val
                        && !allowed.apply(default_val)
                    {
                        return Err(errorf(format!(
                            "Aspect parameter attribute '{native_name}' has a bad default value: \
                             {}",
                            allowed.get_error_reason(default_val)
                        )));
                    }
                }
                if !has_default || attribute.is_mandatory() {
                    required_params.insert(native_name.clone());
                }
            } else if !has_default {
                // Implicit or late bound attribute
                let starlark_name = format!("_{}", &native_name[1..]);
                if attribute.is_late_bound()
                    && !attribute
                        .get_late_bound_default()
                        .is_some_and(|l| l.get_fragment_class().is_some())
                {
                    // Code elsewhere assumes that a late-bound attribute of a Starlark-defined
                    // aspects can exist in Java-land only as a StarlarkLateBoundDefault.
                    return Err(errorf(format!(
                        "Starlark aspect attribute '{starlark_name}' is late-bound but somehow is \
                         not defined in Starlark. This violates an invariant inside of Bazel. \
                         Please file a bug with instructions for reproducing this. Thanks!"
                    )));
                }
                return Err(errorf(format!(
                    "Aspect attribute '{starlark_name}' has no default value."
                )));
            }
            if matches!(
                attribute.get_default_value_unchecked(),
                Some(AttrValue::StarlarkComputedDefaultTemplate(_))
            ) {
                // Attributes specifying dependencies using computed value are currently not
                // supported.
                let starlark_name = format!("_{}", &native_name[1..]);
                return Err(errorf(format!(
                    "Aspect attribute '{starlark_name}' ({}) with computed default value is \
                     unsupported.",
                    attribute.get_type()
                )));
            }
            attributes.push(attribute);
        }

        if apply_to_generating_rules
            && !sequence_cast(
                required_providers_arg,
                "Object",
                |_| true,
                "required_providers",
            )?
            .is_empty()
        {
            return Err(errorf(
                "An aspect cannot simultaneously have required providers and apply to generating \
                 rules.",
            ));
        }

        let mut propagation_predicate_value = None;
        if let Some(raw) = propagation_predicate
            && !raw.is_none()
        {
            if !is_starlark_function(raw) {
                return Err(errorf(format!(
                    "Expected a function in 'propagation_predicate' parameter, got '{}'.",
                    // Java passes the (null) predicate here, whose type is "NoneType".
                    "NoneType"
                )));
            }
            propagation_predicate_value = Some(raw);
        }

        if apply_to_generating_rules && propagation_predicate_value.is_some() {
            return Err(errorf(
                "An aspect cannot simultaneously have a propagation predicate and apply to \
                 generating rules.",
            ));
        }

        let exec_compatible_with = parse_labels(
            raw_exec_compatible_with,
            &label_converter,
            "exec_compatible_with",
        )?;

        let exec_groups = match exec_groups {
            Some(raw) if !raw.is_none() => parse_exec_groups(raw)?,
            _ => IndexMap::new(),
        };

        let mut toolchain_types = parse_toolchain_types(toolchains, &label_converter)?;
        toolchain_types.extend(subrule::discover_toolchains(&subrule_values));

        // Java: AspectPropagationEdgesSupplier.createForAttrAspects / createForToolchainsAspects
        let (attribute_aspects, attr_aspects_function) = if is_starlark_function(attr_aspects) {
            (PropagationEdges::Function, Some(attr_aspects))
        } else {
            (
                PropagationEdges::Fixed(parse_attr_aspects(attr_aspects, true)?),
                None,
            )
        };
        let (toolchains_aspects_edges, toolchains_aspects_function) =
            if is_starlark_function(toolchains_aspects) {
                (PropagationEdges::Function, Some(toolchains_aspects))
            } else {
                (
                    PropagationEdges::Fixed(parse_toolchains_aspects(toolchains_aspects, eval)?),
                    None,
                )
            };

        let required_providers = build_provider_predicate(
            &sequence_cast(
                required_providers_arg,
                "Object",
                |_| true,
                "required_providers",
            )?,
            "required_providers",
        )?;
        let required_aspect_providers = build_provider_predicate(
            &sequence_cast(
                required_aspect_providers_arg,
                "Object",
                |_| true,
                "required_aspect_providers",
            )?,
            "required_aspect_providers",
        )?;
        let provides = get_starlark_provider_identifiers(
            &sequence_cast(provides_arg, "Provider", |_| true, "provides")?,
            "provides",
        )?;
        let required_aspects =
            sequence_cast(requires, "Aspect", |a| is_starlark_aspect(a), "requires")?;
        let fragments: IndexSet<String> = sequence_cast_strings(fragments, "fragments")?
            .into_iter()
            .collect();
        let symbol = blaze_starlark::ThreadContext::get(eval)?.next_symbol();

        Ok(StarlarkDefinedAspectGen {
            implementation,
            attr_aspects_function,
            toolchains_aspects_function,
            propagation_predicate: propagation_predicate_value,
            required_aspects,
            subrules: subrule_values,
            handles: pending.iter().map(|p| p.handle).collect(),
            pending: pending.iter().map(|p| p.value).collect(),
            data: Arc::new(AspectData {
                documentation: doc.map(trim_doc_string),
                attribute_aspects,
                toolchains_aspects: toolchains_aspects_edges,
                attributes,
                required_providers,
                required_aspect_providers,
                provides,
                param_attributes: required_params,
                fragments,
                toolchain_types,
                apply_to_generating_rules,
                exec_compatible_with,
                exec_groups,
                symbol,
                exported: OnceLock::new(),
            }),
        })
    }
}
