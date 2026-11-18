//! Port of `macro()` (`StarlarkRuleClassFunctions.macro`) and
//! `StarlarkRuleClassFunctions.MacroFunction`, the value it returns.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
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

use super::attr_module::as_descriptor;
use super::attr_module::function_parameters;
use super::attr_module::is_starlark_function;
use super::rule_class_functions::check_attribute_name;
use super::rule_class_functions::common_attributes;
use super::rule_class_functions::is_common_attributes_name;
use super::rule_class_functions::rule_function_state;
use super::util::call_stack;
use super::util::dict_entries;
use super::util::errorf;
use super::util::eval_exception;
use super::util::param_type_error;
use super::util::starlark_type;
use super::util::trim_doc_string;
use super::value_cell::PendingValue;
use crate::attribute::Attribute;
use crate::attribute::AttributeValueSource;
use crate::bzl_context::BzlInitThreadContext;
use crate::macros::MacroClass;
use crate::macros::MacroClassBuilder;
use crate::macros::macro_class::RESERVED_MACRO_ATTR_NAMES;
use crate::package::TargetDefinitionContext;

/// The mutable state of a macro function: the builder until export, the class after.
pub struct MacroFunctionState {
    builder: Mutex<Option<MacroClassBuilder>>,
    macro_class: OnceLock<Arc<MacroClass>>,
    exported_location: OnceLock<Location>,
    documentation: Option<String>,
    symbol: u64,
    exported: OnceLock<(Label, String)>,
}

impl fmt::Debug for MacroFunctionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MacroFunctionState")
            .field("exported", &self.exported.get())
            .finish_non_exhaustive()
    }
}

impl MacroFunctionState {
    /// Java `getMacroClass()`: `None` until exported.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1673 MacroFunction.getMacroClass
    pub fn get_macro_class(&self) -> Option<&Arc<MacroClass>> {
        self.macro_class.get()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1568 MacroFunction.getName
    pub fn get_name(&self) -> String {
        match self.macro_class.get() {
            Some(m) => m.get_name().to_owned(),
            None => "unexported macro".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1573 MacroFunction.getLocation
    pub fn get_location(&self) -> Location {
        self.exported_location
            .get()
            .cloned()
            .unwrap_or_else(Location::builtin)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1581 MacroFunction.getDocumentation
    pub fn get_documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1590 MacroFunction.getExtensionLabel
    pub fn get_extension_label(&self) -> Option<&Label> {
        self.exported.get().map(|(l, _)| l)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1678 MacroFunction.isExported
    pub fn is_exported(&self) -> bool {
        self.macro_class.get().is_some()
    }
}

/// A callable Starlark object representing a symbolic macro (Java `MacroFunction`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1542 StarlarkRuleClassFunctions.MacroFunction
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct MacroFunctionGen<V: ValueLifetimeless> {
    /// Values referred to by handles in the macro class (empty once frozen).
    pending: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    handles: Vec<crate::rule_class::StarlarkCallable>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    state: Arc<MacroFunctionState>,
}

starlark_complex_value!(pub MacroFunction);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<MacroFunctionGen<Value<'v>>> for MacroFunctionGen<FrozenValue> {}

impl<'v> Freeze for MacroFunction<'v> {
    type Frozen = FrozenMacroFunction;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenMacroFunction> {
        for (value, handle) in self.pending.into_iter().zip(self.handles) {
            PendingValue { value, handle }.freeze(freezer)?;
        }
        Ok(MacroFunctionGen {
            pending: Vec::new(),
            handles: Vec::new(),
            state: self.state,
        })
    }
}

/// The state of a macro function value, frozen or not (Java `instanceof MacroFunction`).
pub fn macro_function_state(v: Value) -> Option<Arc<MacroFunctionState>> {
    if let Some(m) = MacroFunction::from_value(v) {
        return Some(m.state.clone());
    }
    v.downcast_ref::<FrozenMacroFunction>()
        .map(|m| m.state.clone())
}

impl<'v, V: ValueLike<'v>> MacroFunctionGen<V> {
    pub fn state(&self) -> &Arc<MacroFunctionState> {
        &self.state
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for MacroFunctionGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1683 MacroFunction.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state.macro_class.get() {
            Some(m) => write!(f, "<macro {}>", m.get_name()),
            None => write!(f, "<macro>"),
        }
    }
}

/// Java `MacroFunction.call`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1598 MacroFunction.call
fn call_macro_function<'v>(
    state: &MacroFunctionState,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let mut target_definition_context =
        TargetDefinitionContext::from_or_fail_participle(eval, "a symbolic macro", "instantiated")
            .map_err(eval_exception)?;

    let Some(macro_class) = state.macro_class.get().cloned() else {
        return Err(errorf(
            "Cannot instantiate a macro that has not been exported (assign it to a global \
             variable in the .bzl where it's defined)",
        ));
    };

    if macro_class.is_finalizer() && target_definition_context.currently_in_non_finalizer_macro() {
        return Err(errorf(
            "Cannot instantiate a rule finalizer within a non-finalizer symbolic macro. Rule \
             finalizers may only be instantiated while evaluating a BUILD file, a legacy macro \
             called from a BUILD file, or another rule finalizer.",
        ));
    }

    let named = args.names_map()?;
    if args.len()? != named.len() {
        return Err(errorf("unexpected positional arguments"));
    }
    let kwargs: Vec<(String, Value<'v>)> = named
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();

    let macro_instance = macro_class
        .instantiate_and_add_macro(&mut target_definition_context, &kwargs, &call_stack(eval))
        .map_err(eval_exception)?;

    // Evaluate the macro now, if it's not a finalizer. Finalizer evaluation will be deferred to
    // the end of the BUILD file evaluation.
    //
    // Non-finalizers must be evaluated synchronously with the call to instantiate the macro,
    // because their side-effects must be visible to native.existing_rules() calls in legacy
    // macros.
    if target_definition_context.eagerly_expand_macros() && !macro_class.is_finalizer() {
        // PORT: Java's updatePausedThreadComputationSteps accounting is skipped.
        let semantics = ThreadContext::get(eval)?.semantics.clone();
        MacroClass::execute_macro_implementation(
            &macro_instance,
            &mut target_definition_context,
            &semantics,
        )
        .map_err(eval_exception)?;
    }

    Ok(Value::new_none())
}

#[starlark_value(type = "macro")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for MacroFunctionGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        call_macro_function(&self.state, args, eval)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1692 MacroFunction.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match macro_function_state(other) {
            Some(o) => match (self.state.exported.get(), o.exported.get()) {
                (Some(a), Some(b)) => a == b,
                _ => Arc::ptr_eq(&self.state, &o),
            },
            None => false,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1700 MacroFunction.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.state.exported.get() {
            Some(e) => e.hash(hasher),
            None => self.state.symbol.hash(hasher),
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1651 MacroFunction.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if self.state.macro_class.get().is_some() {
            return Ok(());
        }
        let Some(ctx) = BzlInitThreadContext::from_or_null(eval) else {
            return Ok(());
        };
        let starlark_label = ctx.bzl_file.clone();
        drop(ctx);
        let Some(mut builder) = self.state.builder.lock().take() else {
            return Ok(());
        };
        builder.set_name(variable_name.to_owned());
        builder.set_defining_bzl_label(starlark_label.clone());
        let _ = self.state.macro_class.set(Arc::new(builder.build()));
        let _ = self
            .state
            .exported
            .set((starlark_label, variable_name.to_owned()));
        // PORT: Java receives the location of the exporting assignment; the innermost call-stack
        // location is the closest Blaze has.
        if let Some(loc) = eval.call_stack_top_location() {
            let _ = self
                .state
                .exported_location
                .set(Location::of_span_start(&loc));
        }
        Ok(())
    }
}

/// Java `StarlarkRuleClassFunctions.getAttrsOf`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:507 StarlarkRuleClassFunctions.getAttrsOf
fn get_attrs_of(inherit_attrs_arg: Value) -> starlark::Result<Vec<Attribute>> {
    if inherit_attrs_arg.is_none() {
        return Ok(Vec::new());
    }
    if let Some(state) = rule_function_state(inherit_attrs_arg) {
        verify_inherit_attrs_arg_exported(state.is_exported())?;
        return Ok(state
            .rule_class()
            .expect("exported rule has a rule class")
            .get_attribute_provider()
            .get_attributes()
            .to_vec());
    }
    if let Some(rc) = super::native_rule_class_of(inherit_attrs_arg) {
        // Note that the value of 'inherit_attrs' can be non-exportable (e.g. native rule).
        return Ok(rc.get_attribute_provider().get_attributes().to_vec());
    }
    if let Some(state) = macro_function_state(inherit_attrs_arg) {
        verify_inherit_attrs_arg_exported(state.is_exported())?;
        return Ok(state
            .get_macro_class()
            .unwrap()
            .get_attribute_provider()
            .get_attributes()
            .to_vec());
    }
    if inherit_attrs_arg
        .unpack_str()
        .is_some_and(is_common_attributes_name)
    {
        return Ok(common_attributes());
    }
    Err(errorf(format!(
        "Invalid 'inherit_attrs' value {}; expected a rule, a macro, or \"common\"",
        inherit_attrs_arg.to_repr()
    )))
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:524 StarlarkRuleClassFunctions.verifyInheritAttrsArgExportedIfExportable
fn verify_inherit_attrs_arg_exported(exported: bool) -> starlark::Result<()> {
    if !exported {
        return Err(errorf(
            "Invalid 'inherit_attrs' value: a rule or macro callable must be assigned to a global \
             variable in a .bzl file before it can be inherited from",
        ));
    }
    Ok(())
}

#[starlark_module]
pub fn register_macro(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:415 StarlarkRuleClassFunctions.macro
    fn r#macro<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] attrs: Option<Value<'v>>,
        #[starlark(require = named)] inherit_attrs: Option<Value<'v>>,
        #[starlark(require = named, default = false)] finalizer: bool,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<MacroFunction<'v>> {
        if !is_starlark_function(implementation) {
            return Err(param_type_error(
                "macro",
                "implementation",
                implementation,
                "function",
            ));
        }
        let inherit_attrs = inherit_attrs.unwrap_or_else(Value::new_none);
        let doc = match doc {
            Some(d) if !d.is_none() => match d.unpack_str() {
                Some(s) => Some(s),
                None => return Err(param_type_error("macro", "doc", d, "string or NoneType")),
            },
            _ => None,
        };
        // Ordinarily we would use StarlarkMethod#enableOnlyWithFlag, but this doesn't work for
        // top-level symbols (due to StarlarkGlobalsImpl relying on the Starlark#addMethods
        // overload that uses default StarlarkSemantics), so enforce it here instead.
        if !ThreadContext::get(eval)?
            .semantics
            .experimental_enable_first_class_macros
        {
            return Err(errorf(
                "Use of `macro()` requires --experimental_enable_first_class_macros",
            ));
        }
        // Ensure we're initializing a .bzl file.
        BzlInitThreadContext::from_or_fail(eval, "macro()").map_err(eval_exception)?;

        let implementation_pending = PendingValue::new(implementation, eval);
        let mut pending = vec![implementation_pending];
        let mut builder = MacroClassBuilder::new(implementation_pending.handle);
        let attr_entries = match attrs {
            None => Vec::new(),
            Some(a) => match dict_entries(a) {
                Some(e) => e,
                None => {
                    return Err(param_type_error("macro", "attrs", a, "dict"));
                }
            },
        };
        let mut attr_names: Vec<String> = Vec::new();
        for (key, value) in &attr_entries {
            // Dict.cast() does not support none-able values - so we type-check manually, and
            // translate Starlark None to Java null.
            let descriptor = if value.is_none() {
                None
            } else {
                as_descriptor(*value)
            };
            let Some(attr_name) = key.unpack_str() else {
                return Err(errorf(format!(
                    "got dict<{}, {}> for 'attrs', want dict<string, Attribute|None>",
                    starlark_type(*key),
                    starlark_type(*value)
                )));
            };
            if !value.is_none() && descriptor.is_none() {
                return Err(errorf(format!(
                    "got dict<{}, {}> for 'attrs', want dict<string, Attribute|None>",
                    starlark_type(*key),
                    starlark_type(*value)
                )));
            }
            check_attribute_name(attr_name)?;
            attr_names.push(attr_name.to_owned());

            // "name" and "visibility" attributes are added automatically by the builder.
            if RESERVED_MACRO_ATTR_NAMES.contains(&attr_name) {
                return Err(errorf(format!(
                    "Cannot declare a macro attribute named '{attr_name}'"
                )));
            }

            let Some(descriptor) = descriptor else {
                // a None descriptor should ignored.
                continue;
            };

            if descriptor.get_value_source() != AttributeValueSource::Direct {
                // Note that inherited native attributes may have a computed default, e.g.
                // testonly.
                return Err(errorf(format!(
                    "In macro attribute '{attr_name}': Macros do not support computed defaults or \
                     late-bound defaults"
                )));
            }

            let attr = descriptor.build(attr_name);
            pending.extend(descriptor.pending_values().iter().copied());
            builder.add_attribute(attr);
        }
        for attr in get_attrs_of(inherit_attrs)? {
            let attr_name = attr.get_name().to_owned();
            if attr.is_public()
                // isDocumented() is false only for generator_* magic attrs (for which isPublic()
                // is true)
                && attr.is_documented()
                && !RESERVED_MACRO_ATTR_NAMES.contains(&attr_name.as_str())
                && !attr_names.contains(&attr_name)
            {
                // Force the default value of optional inherited attributes to None.
                let attr = if !attr.is_mandatory() && attr.get_default_value_unchecked().is_some() {
                    let mut b = attr.clone_builder();
                    b.default_value_none();
                    b.build()
                } else {
                    attr
                };
                builder.add_attribute(attr);
            }
        }
        if !inherit_attrs.is_none() && !function_parameters(implementation).1 {
            return Err(errorf(
                "If inherit_attrs is set, implementation function must have a **kwargs parameter",
            ));
        }

        if finalizer {
            builder.set_is_finalizer();
        }

        let symbol = ThreadContext::get(eval)?.next_symbol();
        Ok(MacroFunctionGen {
            handles: pending.iter().map(|p| p.handle).collect(),
            pending: pending.iter().map(|p| p.value).collect(),
            state: Arc::new(MacroFunctionState {
                builder: Mutex::new(Some(builder)),
                macro_class: OnceLock::new(),
                exported_location: OnceLock::new(),
                documentation: doc.map(trim_doc_string),
                symbol,
                exported: OnceLock::new(),
            }),
        })
    }
}
