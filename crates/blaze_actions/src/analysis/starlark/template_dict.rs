//! Port of `lib/analysis/starlark/TemplateDict.java`: the Starlark `TemplateDict` type
//! (`ctx.actions.template_dict()`), a list of substitutions for `ctx.actions.expand_template`'s
//! `computed_substitutions`. Starlark methods and parameters:
//! `lib/starlarkbuildapi/TemplateDictApi.java`.
//!
//! PORT: Java's TemplateDict holds Starlark values (a depset and a `map_each` callable per
//! `add_joined`) and calls `map_each` lazily when the template is expanded. The values must
//! outlive the Starlark heap, so `TemplateDict<'v>` is frozen with the rule implementation's
//! module into [`FrozenTemplateDict`], and [`get_all`] turns the frozen value into
//! [`Substitution`]s whose lazy values keep the frozen heap alive (`OwnedFrozenValue`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_starlark::depset::Depset;
use blaze_starlark::eval::{EvalOptions, frozen_into, with_evaluator};
use blaze_starlark::thread::{Phase, StarlarkEvents, ThreadContext};
use blaze_starlark::{EvalError, Location, StarlarkSemantics};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::none::NoneOr;
use starlark::values::typing::StarlarkCallable;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, OwnedFrozenValue,
    StarlarkValue, Trace, Tracer, UnpackValue, Value, ValueLike, starlark_value,
};

use crate::analysis::actions::substitution::{ComputedSubstitution, Substitution};

use super::args::{is_global_function, is_starlark_function, map_each_not_global_error};
use super::starlark_custom_command_line::java_to_string;

fn err(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// One substitution as recorded by `add` / `add_joined`.
#[derive(Clone)]
enum TemplateItem<V> {
    /// `Substitution.of(key, value)`.
    Plain { key: String, value: String },
    /// A `LazySubstitution`.
    Lazy {
        key: String,
        semantics: Arc<StarlarkSemantics>,
        values_set: V,
        map_each: V,
        uniquify: bool,
        join_with: String,
        format_joined: Option<String>,
    },
}

impl<V> TemplateItem<V> {
    fn try_map<W, E>(self, f: &mut dyn FnMut(V) -> Result<W, E>) -> Result<TemplateItem<W>, E> {
        Ok(match self {
            TemplateItem::Plain { key, value } => TemplateItem::Plain { key, value },
            TemplateItem::Lazy {
                key,
                semantics,
                values_set,
                map_each,
                uniquify,
                join_with,
                format_joined,
            } => TemplateItem::Lazy {
                key,
                semantics,
                values_set: f(values_set)?,
                map_each: f(map_each)?,
                uniquify,
                join_with,
                format_joined,
            },
        })
    }
}

/// Implementation of the `TemplateDict` Starlark type.
// Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:40 TemplateDict
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct TemplateDict<'v> {
    #[allocative(skip)]
    substitutions: RefCell<Vec<TemplateItem<Value<'v>>>>,
}

/// A frozen `TemplateDict`.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTemplateDict {
    #[allocative(skip)]
    substitutions: Vec<TemplateItem<FrozenValue>>,
}

/// Java's default `StarlarkValue.repr` (TemplateDict does not override it).
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
const TEMPLATE_DICT_REPR: &str =
    "<unknown object com.google.devtools.build.lib.analysis.starlark.TemplateDict>";

impl fmt::Debug for TemplateDict<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TemplateDict")
    }
}

impl fmt::Display for TemplateDict<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(TEMPLATE_DICT_REPR)
    }
}

impl fmt::Debug for FrozenTemplateDict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrozenTemplateDict")
    }
}

impl fmt::Display for FrozenTemplateDict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(TEMPLATE_DICT_REPR)
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for TemplateDict<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for item in self.substitutions.get_mut() {
            if let TemplateItem::Lazy {
                values_set,
                map_each,
                ..
            } = item
            {
                values_set.trace(tracer);
                map_each.trace(tracer);
            }
        }
    }
}

impl<'v> Freeze for TemplateDict<'v> {
    type Frozen = FrozenTemplateDict;

    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenTemplateDict> {
        let mut substitutions = Vec::new();
        for item in self.substitutions.into_inner() {
            substitutions.push(item.try_map(&mut |v: Value<'v>| freezer.freeze(v))?);
        }
        Ok(FrozenTemplateDict { substitutions })
    }
}

impl<'v> AllocValue<'v> for TemplateDict<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "TemplateDict")]
impl<'v> StarlarkValue<'v> for TemplateDict<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("TemplateDict", template_dict_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(TEMPLATE_DICT_REPR);
    }
}

#[starlark_value(type = "TemplateDict")]
impl<'v> StarlarkValue<'v> for FrozenTemplateDict {
    type Canonical = TemplateDict<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("TemplateDict", template_dict_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(TEMPLATE_DICT_REPR);
    }
}

/// `TemplateDict.newDict()`: allocates the value returned by `ctx.actions.template_dict()`.
// Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:46 TemplateDict.newDict
pub fn new_template_dict<'v>(heap: Heap<'v>) -> Value<'v> {
    heap.alloc(TemplateDict {
        substitutions: RefCell::new(Vec::new()),
    })
}

/// A copy of a `TemplateDict` value as it is at this call, as a new `TemplateDict` on `heap`.
/// Later `add` / `add_joined` calls on `dict` do not reach the copy.
///
/// Java reads the dict when `ctx.actions.expand_template` is called (`TemplateDict.getAll()`
/// goes into the action's substitutions then). The substitutions can only be read from a frozen
/// value here ([`get_all`]), so the caller takes this copy at the call and reads the copy after
/// freezing. A value that is not a mutable `TemplateDict` is returned as it is.
// Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:91 TemplateDict.getAll
pub fn snapshot_template_dict<'v>(dict: Value<'v>, heap: Heap<'v>) -> Value<'v> {
    match dict.downcast_ref::<TemplateDict>() {
        Some(d) => heap.alloc(TemplateDict {
            substitutions: RefCell::new(d.substitutions.borrow().clone()),
        }),
        None => dict,
    }
}

/// Java `x instanceof TemplateDict`.
pub fn is_template_dict(v: Value) -> bool {
    v.downcast_ref::<TemplateDict>().is_some() || v.downcast_ref::<FrozenTemplateDict>().is_some()
}

/// `TemplateDict.getAll()` of a frozen TemplateDict value. Returns `Err` if the value is not a
/// frozen TemplateDict.
// Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:91 TemplateDict.getAll
pub fn get_all(dict: &OwnedFrozenValue) -> Result<Vec<Substitution>, String> {
    let v = dict.value();
    let Some(frozen) = v.downcast_ref::<FrozenTemplateDict>() else {
        if v.downcast_ref::<TemplateDict>().is_some() {
            return Err("TemplateDict value is not frozen yet".to_owned());
        }
        return Err(format!("expected a TemplateDict, got '{}'", v.get_type()));
    };
    let mut result = Vec::with_capacity(frozen.substitutions.len());
    for item in &frozen.substitutions {
        result.push(match item {
            TemplateItem::Plain { key, value } => Substitution::of(key.clone(), value.clone()),
            TemplateItem::Lazy {
                key,
                semantics,
                values_set,
                map_each,
                uniquify,
                join_with,
                format_joined,
            } => {
                let values_set = *values_set;
                let map_each = *map_each;
                Substitution::computed(Arc::new(LazySubstitution {
                    key: key.clone(),
                    semantics: semantics.clone(),
                    values_set: dict.map(|_| values_set),
                    map_each: dict.map(|_| map_each),
                    uniquify: *uniquify,
                    join_with: join_with.clone(),
                    format_joined: format_joined.clone(),
                }))
            }
        });
    }
    Ok(result)
}

/// A substitution whose value is computed by calling `map_each` on each element of a depset.
// Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:95 TemplateDict.LazySubstitution
#[derive(Debug)]
struct LazySubstitution {
    key: String,
    semantics: Arc<StarlarkSemantics>,
    values_set: OwnedFrozenValue,
    map_each: OwnedFrozenValue,
    uniquify: bool,
    join_with: String,
    format_joined: Option<String>,
}

/// The map_each callback should not create any persistent state beyond the returned String
/// value; its events are dropped.
struct NoopEvents;

impl StarlarkEvents for NoopEvents {
    fn debug(&self, _location: Option<&Location>, _message: &str) {}

    fn warn(&self, _location: Option<&Location>, _message: &str) {}
}

/// Java `ret instanceof Sequence`.
fn is_sequence(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range")
}

impl ComputedSubstitution for LazySubstitution {
    fn get_key(&self) -> &str {
        &self.key
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/TemplateDict.java:122 LazySubstitution.getValue
    fn get_value(&self) -> Result<String, String> {
        // PORT: Java prints are not routed anywhere specific for this thread ("map_each
        // callback"); events are dropped here.
        let ctx = ThreadContext::new(Phase::Other, self.semantics.clone(), Arc::new(NoopEvents));
        let outcome = with_evaluator(&ctx, None, &EvalOptions::default(), |module, eval| {
            let values_set = frozen_into(module, &self.values_set);
            let values = Depset::from_value(values_set)
                .expect("depset")
                .to_list()
                .map_err(|e| EvalError::from_starlark(&e))?;
            let map_each = frozen_into(module, &self.map_each);
            let mut parts: Vec<String> = Vec::with_capacity(values.len());
            for val in values {
                let ret = eval
                    .eval_function(map_each, &[val], &[])
                    .map_err(|e| EvalError::from_starlark(&e))?;
                if let Some(string) = ret.unpack_str() {
                    parts.push(string.to_owned());
                } else if is_sequence(ret) {
                    let items = ret
                        .iterate(eval.heap())
                        .map_err(|e| EvalError::from_starlark(&e))?;
                    for v in items {
                        match v.unpack_str() {
                            Some(s) => parts.push(s.to_owned()),
                            None => {
                                return Ok(Err(format!(
                                    "Function provided to map_each must return string, None, or \
                                     list of strings, but returned list containing element '{}' \
                                     of type {} for key '{}' and value: {}",
                                    java_to_string(v),
                                    v.get_type(),
                                    self.key,
                                    java_to_string(val)
                                )));
                            }
                        }
                    }
                } else if !ret.is_none() {
                    return Ok(Err(format!(
                        "Function provided to map_each must return string, None, or list of \
                         strings, but returned type {} for key '{}' and value: {}",
                        ret.get_type(),
                        self.key,
                        java_to_string(val)
                    )));
                }
            }
            if self.uniquify {
                // Stably deduplicate parts in-place.
                let mut seen: HashSet<String> = HashSet::with_capacity(parts.len());
                parts.retain(|p| seen.insert(p.clone()));
            }
            let joined = parts.join(&self.join_with);
            if let Some(format_joined) = &self.format_joined {
                // Java: Starlark.format(semantics, formatJoined, joined)
                let heap = eval.heap();
                let formatted = heap
                    .alloc_str(format_joined)
                    .to_value()
                    .percent(heap.alloc_str(&joined).to_value(), heap)
                    .map_err(|e| EvalError::from_starlark(&e))?;
                return Ok(Ok(formatted.unpack_str().unwrap_or_default().to_owned()));
            }
            Ok(Ok(joined))
        });
        match outcome {
            Ok(result) => result,
            // Java propagates the EvalException; its message is what callers report.
            Err(e) => Err(e.message),
        }
    }
}

fn mutable_dict<'v>(this: Value<'v>) -> starlark::Result<&'v TemplateDict<'v>> {
    match this.downcast_ref::<TemplateDict<'v>>() {
        Some(d) => Ok(d),
        // PORT: Java's TemplateDict has no Mutability and stays mutable; here a TemplateDict
        // frozen with its module cannot change.
        None => Err(err("trying to mutate a frozen TemplateDict value")),
    }
}

#[starlark_module]
fn template_dict_methods(builder: &mut MethodsBuilder) {
    /// Java: TemplateDictApi.addArgument (TemplateDict.java:52)
    fn add<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: &str,
        #[starlark(require = pos)] value: &str,
    ) -> starlark::Result<Value<'v>> {
        let dict = mutable_dict(this)?;
        dict.substitutions.borrow_mut().push(TemplateItem::Plain {
            key: key.to_owned(),
            value: value.to_owned(),
        });
        Ok(this)
    }

    /// Java: TemplateDictApi.addJoined (TemplateDict.java:60)
    fn add_joined<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: &str,
        #[starlark(require = pos)] values: Value<'v>,
        #[starlark(require = named)] join_with: &str,
        #[starlark(require = named)] map_each: Value<'v>,
        #[starlark(require = named, default = false)] uniquify: bool,
        #[starlark(require = named, default = NoneOr::None)] format_joined: NoneOr<&str>,
        #[starlark(require = named, default = false)] allow_closure: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        if Depset::from_value(values).is_none() {
            return Err(err(format!(
                "in call to add_joined(), parameter 'values' got value of type '{}', want \
                 'depset'",
                values.get_type()
            )));
        }
        if <StarlarkCallable<'v>>::unpack_value_opt(map_each).is_none() {
            return Err(err(format!(
                "in call to add_joined(), parameter 'map_each' got value of type '{}', want \
                 'callable'",
                map_each.get_type()
            )));
        }
        let dict = mutable_dict(this)?;
        if is_starlark_function(map_each) {
            if !allow_closure && !is_global_function(map_each, eval) {
                return Err(map_each_not_global_error(map_each));
            }
        }
        let semantics = ThreadContext::get(eval)?.semantics.clone();
        dict.substitutions.borrow_mut().push(TemplateItem::Lazy {
            key: key.to_owned(),
            semantics,
            values_set: values,
            map_each,
            uniquify,
            join_with: join_with.to_owned(),
            format_joined: format_joined.into_option().map(str::to_owned),
        });
        Ok(this)
    }
}
