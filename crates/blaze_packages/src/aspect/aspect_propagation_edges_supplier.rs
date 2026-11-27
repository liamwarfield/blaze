//! Port of `packages/AspectPropagationEdgesSupplier.java`.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_starlark::EvalError;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::eval::call_frozen;
use blaze_starlark::label::LabelValue;
use blaze_starlark::thread::CollectingEvents;
use indexmap::IndexSet;
use starlark::environment::Module;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::ListRef;

use crate::attribute::AttributeValueSource;
use crate::rule_class::StarlarkCallable;
use crate::starlark_defs::util::errorf;
use crate::starlark_defs::util::eval_exception;
use crate::starlark_defs::util::sequence_cast;
use crate::starlark_defs::util::sequence_cast_strings;
use crate::starlark_defs::value_cell::same_object;
use crate::types::LabelConverter;

/// The Starlark context object passed to propagation functions (Java
/// `StarlarkAspectPropagationContextApi`, implemented by analysis'
/// `StarlarkAspectPropagationContext`).
///
/// The caller allocates it on the heap of the transient module the function runs in: the
/// closure gets that module (use `blaze_starlark::eval::frozen_into` to bring frozen values in)
/// and returns the context value.
pub type StarlarkAspectPropagationContext<'a> = &'a dyn for<'v> Fn(&Module<'v>) -> Value<'v>;

/// Java `AspectPropagationEdgesSupplier.ALL_TOOLCHAINS`: the pseudo-label standing for "all
/// toolchain types" in `toolchains_aspects`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:48 AspectPropagationEdgesSupplier.ALL_TOOLCHAINS
pub static ALL_TOOLCHAINS: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked("//__toolchains_aspects__:all"));

/// Supplies the list of edges (attribute names or toolchain types) that should be propagated by
/// an aspect (Java `AspectPropagationEdgesSupplier<T>`): a list fixed and known at the aspect
/// definition time, or a function computing it for each target the aspect visits.
///
/// `T` is `String` for `attr_aspects` and [`Label`] for `toolchains_aspects`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:46 AspectPropagationEdgesSupplier
#[derive(Clone, PartialEq, Eq)]
pub enum AspectPropagationEdgesSupplier<T: std::hash::Hash + Eq> {
    /// Java `FixedListSupplier`.
    FixedList(FixedListSupplier<T>),
    /// Java `FunctionSupplier` (`AttrAspectsFunctionSupplier` for `String`,
    /// `ToolchainsAspectsFunctionSupplier` for `Label`).
    Function(FunctionSupplier<T>),
}

impl<T: std::hash::Hash + Eq + fmt::Debug> fmt::Debug for AspectPropagationEdgesSupplier<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AspectPropagationEdgesSupplier::FixedList(s) => {
                f.debug_tuple("FixedListSupplier").field(&s.edges).finish()
            }
            AspectPropagationEdgesSupplier::Function(s) => f
                .debug_tuple("FunctionSupplier")
                .field(&s.function)
                .finish(),
        }
    }
}

impl<T: std::hash::Hash + Eq> AspectPropagationEdgesSupplier<T> {
    /// Java `instanceof FixedListSupplier` then `getList()`.
    pub fn as_fixed_list(&self) -> Option<&IndexSet<T>> {
        match self {
            AspectPropagationEdgesSupplier::FixedList(s) => Some(s.get_list()),
            AspectPropagationEdgesSupplier::Function(_) => None,
        }
    }
}

/// Java `AspectPropagationEdgesSupplier.DEFAULT_ATTR_ASPECTS_SUPPLIER`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:50 AspectPropagationEdgesSupplier.DEFAULT_ATTR_ASPECTS_SUPPLIER
pub fn default_attr_aspects_supplier() -> AspectPropagationEdgesSupplier<String> {
    AspectPropagationEdgesSupplier::FixedList(FixedListSupplier {
        edges: IndexSet::new(),
    })
}

/// Java `AspectPropagationEdgesSupplier.DEFAULT_TOOLCHAINS_ASPECTS_SUPPLIER`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:53 AspectPropagationEdgesSupplier.DEFAULT_TOOLCHAINS_ASPECTS_SUPPLIER
pub fn default_toolchains_aspects_supplier() -> AspectPropagationEdgesSupplier<Label> {
    AspectPropagationEdgesSupplier::FixedList(FixedListSupplier {
        edges: IndexSet::new(),
    })
}

/// A supplier of the edges that is fixed and known at the aspect definition time.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:57 AspectPropagationEdgesSupplier.FixedListSupplier
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedListSupplier<T: std::hash::Hash + Eq> {
    edges: IndexSet<T>,
}

impl<T: std::hash::Hash + Eq> FixedListSupplier<T> {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:65 AspectPropagationEdgesSupplier.FixedListSupplier.getList
    pub fn get_list(&self) -> &IndexSet<T> {
        &self.edges
    }
}

/// A supplier of the edges that is computed for each target that aspect visits.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:88 AspectPropagationEdgesSupplier.FunctionSupplier
pub struct FunctionSupplier<T> {
    function: StarlarkCallable,
    semantics: Arc<StarlarkSemantics>,
    _edge: PhantomData<fn() -> T>,
}

/// A function supplier for `attr_aspects`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:142 AspectPropagationEdgesSupplier.AttrAspectsFunctionSupplier
pub type AttrAspectsFunctionSupplier = FunctionSupplier<String>;

/// A function supplier for `toolchains_aspects`.
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:156 AspectPropagationEdgesSupplier.ToolchainsAspectsFunctionSupplier
pub type ToolchainsAspectsFunctionSupplier = FunctionSupplier<Label>;

impl<T> Clone for FunctionSupplier<T> {
    fn clone(&self) -> Self {
        FunctionSupplier {
            function: self.function,
            semantics: self.semantics.clone(),
            _edge: PhantomData,
        }
    }
}

impl<T> PartialEq for FunctionSupplier<T> {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:103 AspectPropagationEdgesSupplier.FunctionSupplier.equals
    // PORT: `StarlarkFunction` equality is identity.
    fn eq(&self, other: &FunctionSupplier<T>) -> bool {
        same_object(self.function, other.function) && self.semantics == other.semantics
    }
}

impl<T> Eq for FunctionSupplier<T> {}

impl<T> FunctionSupplier<T> {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:93 AspectPropagationEdgesSupplier.FunctionSupplier.<init>
    pub fn new(function: StarlarkCallable, semantics: Arc<StarlarkSemantics>) -> Self {
        FunctionSupplier {
            function,
            semantics,
            _edge: PhantomData,
        }
    }

    /// The Starlark function.
    pub fn get_function(&self) -> StarlarkCallable {
        self.function
    }

    pub fn get_semantics(&self) -> &Arc<StarlarkSemantics> {
        &self.semantics
    }

    /// Java `runFunction`: calls the function with the context in a transient thread and
    /// requires a list result. `convert` turns the list (Java `StarlarkList`) into Rust data;
    /// its errors are Java's exceptions thrown after the call (no Starlark stack).
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:125 AspectPropagationEdgesSupplier.FunctionSupplier.runFunction
    fn run_function<R>(
        &self,
        context: StarlarkAspectPropagationContext<'_>,
        event_handler: &dyn EventHandler,
        convert: impl for<'v> FnOnce(Value<'v>) -> starlark::Result<R>,
    ) -> Result<R, EvalError> {
        run_transient_call(
            self.function,
            &self.semantics,
            context,
            event_handler,
            |starlark_result| {
                if ListRef::from_value(starlark_result).is_some() {
                    return convert(starlark_result).map_err(|e| eval_exception_of(&e));
                }
                Err(eval_exception_message("Expected a list"))
            },
        )
    }
}

impl FunctionSupplier<String> {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:148 AspectPropagationEdgesSupplier.AttrAspectsFunctionSupplier.computeList
    pub fn compute_list(
        &self,
        context: StarlarkAspectPropagationContext<'_>,
        event_handler: &dyn EventHandler,
    ) -> Result<IndexSet<String>, EvalError> {
        self.run_function(context, event_handler, |list| {
            parse_attr_aspects(list, /* allow_all= */ false)
        })
    }
}

impl FunctionSupplier<Label> {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:163 AspectPropagationEdgesSupplier.ToolchainsAspectsFunctionSupplier.computeList
    pub fn compute_list(
        &self,
        context: StarlarkAspectPropagationContext<'_>,
        event_handler: &dyn EventHandler,
    ) -> Result<IndexSet<Label>, EvalError> {
        self.run_function(context, event_handler, |list_result| {
            if ListRef::from_value(list_result).is_none_or(|l| l.is_empty()) {
                return Ok(IndexSet::new());
            }
            let elems = sequence_cast(
                list_result,
                "Label",
                |e| e.downcast_ref::<LabelValue>().is_some(),
                "toolchains_aspects",
            )?;
            Ok(elems
                .into_iter()
                .map(|e| {
                    e.downcast_ref::<LabelValue>()
                        .expect("checked above")
                        .label()
                        .clone()
                })
                .collect())
        })
    }
}

/// Runs `function(context)` in a transient thread (Java `StarlarkThread.createTransient` with a
/// debug print handler on `event_handler` and an empty `StarlarkThreadContext`), passing the
/// result to `extract`. Shared by [`FunctionSupplier`] and
/// [`super::AspectPropagationPredicate`].
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:125 AspectPropagationEdgesSupplier.FunctionSupplier.runFunction
pub fn run_transient_call<R>(
    function: StarlarkCallable,
    semantics: &Arc<StarlarkSemantics>,
    context: StarlarkAspectPropagationContext<'_>,
    event_handler: &dyn EventHandler,
    extract: impl for<'v> FnOnce(Value<'v>) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    let events = Arc::new(CollectingEvents::default());
    // Java: com/google/devtools/build/lib/packages/AspectPropagationPredicate.java:65 AspectPropagationPredicate.AspectPropagationThreadContext.<init>
    // PORT: Java stores an AspectPropagationEdgesThreadContext / AspectPropagationThreadContext
    // (no fields) in the thread; no builtin reads it, so the phase is `Other`.
    let ctx = ThreadContext::new(Phase::Other, semantics.clone(), events.clone());
    let result = call_frozen(
        &function.to_owned_frozen_value(),
        &ctx,
        &EvalOptions::default(),
        // Java: Starlark.positionalOnlyCall(thread, function, context)
        |module| (vec![context(module)], Vec::new()),
        |res, _module, _eval| extract(res),
    );
    // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(eventHandler))
    for e in events.take() {
        let location = e
            .location
            .as_ref()
            .map(|l| blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32));
        let event = if e.kind == "DEBUG" {
            blaze_events::Event::debug_at(location, e.message)
        } else {
            blaze_events::Event::warn_at(location, e.message)
        };
        event_handler.handle(&event);
    }
    result
}

/// Java `new EvalException(message)` thrown outside the Starlark call (no stack).
pub fn eval_exception_message(message: &str) -> EvalError {
    EvalError {
        kind: EvalErrorKind::Eval,
        message: message.to_owned(),
        callstack: Vec::new(),
        include_stack: true,
    }
}

fn eval_exception_of(e: &starlark::Error) -> EvalError {
    eval_exception_message(&blaze_starlark::error::bare_message(e))
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:174 AspectPropagationEdgesSupplier.createForAttrAspects
pub fn create_for_attr_aspects(
    raw_attr_aspects: Value,
    function: Option<StarlarkCallable>,
    semantics: &Arc<StarlarkSemantics>,
) -> starlark::Result<AspectPropagationEdgesSupplier<String>> {
    // PORT: the caller passes the keep-alive handle of `raw_attr_aspects` when it is a
    // `StarlarkFunction`.
    match function {
        Some(attr_aspects_function) => Ok(AspectPropagationEdgesSupplier::Function(
            FunctionSupplier::new(attr_aspects_function, semantics.clone()),
        )),
        None => Ok(AspectPropagationEdgesSupplier::FixedList(
            FixedListSupplier {
                edges: parse_attr_aspects(raw_attr_aspects, /* allow_all= */ true)?,
            },
        )),
    }
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:183 AspectPropagationEdgesSupplier.createForToolchainsAspects
pub fn create_for_toolchains_aspects(
    raw_toolchains_aspects: Value,
    function: Option<StarlarkCallable>,
    semantics: &Arc<StarlarkSemantics>,
    label_converter: &LabelConverter,
) -> starlark::Result<AspectPropagationEdgesSupplier<Label>> {
    match function {
        Some(toolchains_aspects_function) => Ok(AspectPropagationEdgesSupplier::Function(
            FunctionSupplier::new(toolchains_aspects_function, semantics.clone()),
        )),
        None => Ok(AspectPropagationEdgesSupplier::FixedList(
            FixedListSupplier {
                edges: parse_toolchains_aspects(raw_toolchains_aspects, label_converter)?,
            },
        )),
    }
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:194 AspectPropagationEdgesSupplier.parseAttrAspects
fn parse_attr_aspects(
    raw_attr_aspects: Value,
    allow_all: bool,
) -> starlark::Result<IndexSet<String>> {
    let attr_aspects = sequence_cast_strings(raw_attr_aspects, "attr_aspects")?;
    let mut attr_aspects_builder = IndexSet::new();
    for attr_name in &attr_aspects {
        if attr_name == "*" {
            if !allow_all {
                return Err(errorf("'*' is not allowed in 'attr_aspects' list"));
            } else if attr_aspects.len() != 1 {
                return Err(errorf("'*' must be the only string in 'attr_aspects' list"));
            }
        }
        if !attr_name.starts_with('_') {
            attr_aspects_builder.insert(attr_name.clone());
        } else {
            // Implicit attribute names mean either implicit or late-bound attributes
            // (``$attr`` or ``:attr``). Depend on both.
            attr_aspects_builder.insert(
                AttributeValueSource::ComputedDefault
                    .convert_to_native_name(attr_name)
                    .map_err(eval_exception)?,
            );
            attr_aspects_builder.insert(
                AttributeValueSource::LateBound
                    .convert_to_native_name(attr_name)
                    .map_err(eval_exception)?,
            );
        }
    }
    Ok(attr_aspects_builder)
}

// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:221 AspectPropagationEdgesSupplier.parseToolchainsAspects
fn parse_toolchains_aspects(
    raw_toolchains_aspects: Value,
    label_converter: &LabelConverter,
) -> starlark::Result<IndexSet<Label>> {
    let toolchains_aspects = sequence_cast_strings(raw_toolchains_aspects, "toolchains_aspects")?;

    if toolchains_aspects.len() == 1 && toolchains_aspects[0] == "*" {
        return Ok(IndexSet::from([ALL_TOOLCHAINS.clone()]));
    }

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
                    "Unable to parse label '{input}' in attribute '{}': {}",
                    "toolchains_aspects",
                    e.get_message()
                )));
            }
        }
    }
    Ok(parsed_labels)
}
