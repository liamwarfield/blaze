//! Port of `macro()` (`StarlarkRuleClassFunctions.macro`) and
//! `StarlarkRuleClassFunctions.MacroFunction`, the value it returns.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_starlark::EvalError;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::error::EvalErrorWithStack;
use blaze_starlark::error::StackEntry;
use blaze_starlark::error::bare_message;
use indexmap::IndexMap;
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
    /// The source line at `exported_location`, for tracebacks.
    exported_source_line: OnceLock<String>,
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

/// Java `thread.getCallStack()` from inside a macro function: [`call_stack`] with the macro's
/// own frame at its location (Java `StarlarkThread.push` gives a callable's frame
/// `fn.getLocation()`, which for an exported macro is where it was exported).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1573 MacroFunction.getLocation
fn macro_call_stack(state: &MacroFunctionState, eval: &Evaluator) -> Vec<StackEntry> {
    let mut stack = call_stack(eval);
    if let (Some(last), Some(location)) = (stack.last_mut(), state.exported_location.get()) {
        last.location = Some(location.clone());
        last.source_line = state.exported_source_line.get().cloned();
    }
    stack
}

/// Java `MacroFunction.call`.
///
/// An `EvalException` thrown from `call` gets the thread's call stack (Java
/// `EvalException.ensureStack`), whose innermost frame is the macro at its location; an
/// exception of the macro's implementation already has its stack
/// (MacroClass.executeMacroImplementation).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1598 MacroFunction.call
fn call_macro_function<'v>(
    state: &MacroFunctionState,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    match call_macro_function_impl(state, args, eval) {
        Ok(v) => Ok(v),
        Err(e) => {
            if EvalErrorWithStack::of(&e).is_some() || state.exported_location.get().is_none() {
                return Err(e);
            }
            let mut ex = EvalError {
                kind: EvalErrorKind::Eval,
                message: bare_message(&e),
                callstack: macro_call_stack(state, eval),
                include_stack: true,
            };
            ex.read_missing_source_lines();
            Err(EvalErrorWithStack(ex).into_starlark())
        }
    }
}

fn call_macro_function_impl<'v>(
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
    let kwargs: IndexMap<String, Value<'v>> = named
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();

    let macro_instance = macro_class
        .instantiate_and_add_macro(
            &mut target_definition_context,
            &kwargs,
            &macro_call_stack(state, eval),
            eval.heap(),
        )
        .map_err(eval_exception)?;

    // Evaluate the macro now, if it's not a finalizer. Finalizer evaluation will be deferred to
    // the end of the BUILD file evaluation.
    //
    // Non-finalizers must be evaluated synchronously with the call to instantiate the macro,
    // because their side-effects must be visible to native.existing_rules() calls in legacy
    // macros.
    //
    // TODO: #19922 - Once compatibility with native.existing_rules() in legacy macros is no
    // longer a concern, we can make all symbolic macros use deferred evaluation rather than
    // expanding them here. And when we have lazy evaluation, they won't even be expanded at the
    // end of BUILD file evaluation, but rather at the end of package evaluation (which at that
    // time would be a distinct skyfunction).
    let eagerly_expand_macros = target_definition_context.eagerly_expand_macros();
    drop(target_definition_context);
    if eagerly_expand_macros && !macro_class.is_finalizer() {
        // PORT: Java's updatePausedThreadComputationSteps accounting is skipped.
        // PORT: the builder is this thread's payload; it moves into the macro's thread while
        // the implementation runs (MacroClass.executeMacroImplementation) and comes back.
        let thread = ThreadContext::get(eval)?;
        let semantics = thread.semantics.clone();
        let builder = thread
            .clear_payload()
            .and_then(|p| p.downcast::<TargetDefinitionContext>().ok())
            .expect("the thread context holds the package builder");
        let (builder, result) =
            MacroClass::execute_macro_implementation(&macro_instance, *builder, &semantics);
        thread.restore_payload(Some(builder_box(builder)));
        result.map_err(|e| EvalErrorWithStack(e).into_starlark())?;
    }

    Ok(Value::new_none())
}

fn builder_box(builder: TargetDefinitionContext) -> Box<dyn std::any::Any> {
    Box::new(builder)
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

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1568 MacroFunction.getName
    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        self.state.get_name()
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
        // Java receives the location of the assigned identifier (BzlLoadFunction's post-assign
        // hook). PORT: starlark-rust's export hook has no location; it is found by the
        // assignment of `variable_name` among the top-level statements of the module being
        // executed (the bottom frame while its top level runs).
        if let Some(module) = eval.innermost_enclosing_def_module(0)
            && let Some((location, source_line)) =
                top_level_assignment_location(module.codemap(), variable_name)
        {
            let _ = self.state.exported_source_line.set(source_line);
            let _ = self.state.exported_location.set(location);
        }
        Ok(())
    }
}

/// The location of the identifier `name` in the first top-level assignment to it in the file of
/// `codemap`, and the trimmed source line there.
fn top_level_assignment_location(
    codemap: &starlark::codemap::CodeMap,
    name: &str,
) -> Option<(Location, String)> {
    use starlark::syntax::ast::AssignTargetP;
    use starlark::syntax::ast::AstAssignTarget;
    use starlark::syntax::ast::StmtP;

    fn find(target: &AstAssignTarget, name: &str) -> Option<starlark::codemap::Span> {
        match &target.node {
            AssignTargetP::Identifier(id) if id.node.ident == name => Some(id.span),
            AssignTargetP::Tuple(items) => items.iter().find_map(|t| find(t, name)),
            _ => None,
        }
    }

    let ast = starlark::syntax::AstModule::parse(
        codemap.filename(),
        codemap.source().to_owned(),
        &blaze_starlark::FileKind::Bzl.dialect(),
    )
    .ok()?;
    let top = ast.statement();
    let statements: Vec<_> = match &top.node {
        StmtP::Statements(list) => list.iter().collect(),
        _ => vec![top],
    };
    let span = statements.iter().find_map(|stmt| match &stmt.node {
        StmtP::Assign(assign) => find(&assign.lhs, name),
        _ => None,
    })?;
    let file_span = ast.file_span(span);
    let location = Location::of_span_start(&file_span);
    let source_line = file_span
        .file
        .source_line(location.line.saturating_sub(1))
        .trim_matches(|c: char| c <= ' ')
        .to_owned();
    Some((location, source_line))
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
        #[starlark(require = named)] finalizer: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<MacroFunction<'v>> {
        // Java's BuiltinFunction checks the parameters' types (ParamDescriptor allowedTypes,
        // StarlarkRuleFunctionsApi.java:202) before the method body runs, parameter by parameter.
        // PORT: Java checks named arguments in the order of the call (BuiltinFunction.java:376
        // addNamedArg); starlark-rust gives the function no call order, so they are checked in
        // the order of declaration, which differs only for a call with two ill-typed arguments.
        if !is_starlark_function(implementation) {
            return Err(param_type_error(
                "macro",
                "implementation",
                implementation,
                "function",
            ));
        }
        if let Some(a) = attrs {
            if dict_entries(a).is_none() {
                return Err(param_type_error("macro", "attrs", a, "dict"));
            }
        }
        let inherit_attrs = inherit_attrs.unwrap_or_else(Value::new_none);
        // Java: @ParamType RuleFunctionApi, MacroFunctionApi, String, NoneType.
        if !(inherit_attrs.is_none()
            || inherit_attrs.unpack_str().is_some()
            || rule_function_state(inherit_attrs).is_some()
            || super::native_rule_class_of(inherit_attrs).is_some()
            || macro_function_state(inherit_attrs).is_some())
        {
            return Err(param_type_error(
                "macro",
                "inherit_attrs",
                inherit_attrs,
                "rule, macro, string, or NoneType",
            ));
        }
        let finalizer = match finalizer {
            None => false,
            Some(f) => match f.unpack_bool() {
                Some(b) => b,
                None => return Err(param_type_error("macro", "finalizer", f, "bool")),
            },
        };
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
            // Checked above.
            Some(a) => dict_entries(a).expect("attrs is a dict"),
        };
        let mut attr_names: Vec<String> = Vec::new();
        for (key, value) in &attr_entries {
            // Dict.cast() does not support none-able values - so we type-check manually, and
            // translate Starlark None to Java null.
            // Java: the key's cast, checkAttributeName, then the value's cast, all inside the
            // try whose ClassCastException becomes the dict<K, V> error.
            let Some(attr_name) = key.unpack_str() else {
                return Err(errorf(format!(
                    "got dict<{}, {}> for 'attrs', want dict<string, Attribute|None>",
                    starlark_type(*key),
                    starlark_type(*value)
                )));
            };
            check_attribute_name(attr_name)?;
            let descriptor = if value.is_none() {
                None
            } else {
                as_descriptor(*value)
            };
            if !value.is_none() && descriptor.is_none() {
                return Err(errorf(format!(
                    "got dict<{}, {}> for 'attrs', want dict<string, Attribute|None>",
                    starlark_type(*key),
                    starlark_type(*value)
                )));
            }
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
                exported_source_line: OnceLock::new(),
                documentation: doc.map(trim_doc_string),
                symbol,
                exported: OnceLock::new(),
            }),
        })
    }
}
