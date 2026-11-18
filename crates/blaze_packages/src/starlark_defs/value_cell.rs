//! Handles for Starlark values that model objects (rule classes, attributes, aspects lists,
//! transition factories) must hold before the defining .bzl module is frozen.
//!
//! PORT: Java model objects hold the live `StarlarkFunction`/`StarlarkAspect`/... object. In
//! Blaze they hold a [`StarlarkCallable`] (a `FrozenValue`, DESIGN.md "Rule classes from
//! rule()"). A `RuleClass` is built at export time (Java builds it in `export`), while its
//! implementation function and the rest are still unfrozen values of the module being
//! evaluated. So the builtins allocate a [`StarlarkValueCell`] on the module's frozen heap
//! (`Evaluator::frozen_heap`, which becomes part of the frozen module and is kept alive by
//! BZL_LOAD — PORT: keep-alive), hand out a `StarlarkCallable` pointing at the cell, and fill the
//! cell with the frozen value when the Starlark object holding the original value is frozen
//! ([`PendingValue`]). Values that are already frozen (loaded from another .bzl) are used
//! directly.
//!
//! Model code must call [`resolve`] (or `StarlarkCallable::value`, once A1 makes it unwrap
//! cells) before downcasting. Calling the cell forwards to the value, so `call_frozen` on an
//! unresolved handle also works.

use std::fmt;
use std::sync::OnceLock;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use crate::rule_class::StarlarkCallable;

/// A frozen slot that receives a value's frozen form when the defining module freezes.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkValueCell {
    #[allocative(skip)]
    inner: OnceLock<FrozenValue>,
}

starlark_simple_value!(StarlarkValueCell);

impl fmt::Debug for StarlarkValueCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.get() {
            Some(v) => write!(f, "StarlarkValueCell({v})"),
            None => write!(f, "StarlarkValueCell(<unfrozen>)"),
        }
    }
}

impl fmt::Display for StarlarkValueCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.get() {
            Some(v) => write!(f, "{v}"),
            None => write!(f, "<unfrozen value>"),
        }
    }
}

impl StarlarkValueCell {
    /// The frozen value, once the module holding the original was frozen.
    pub fn get(&self) -> Option<FrozenValue> {
        self.inner.get().copied()
    }

    fn fill(&self, v: FrozenValue) {
        // Idempotent: freezing memoizes, so every holder fills the same value.
        let _ = self.inner.set(v);
    }
}

#[starlark_value(type = "StarlarkValueCell")]
impl<'v> StarlarkValue<'v> for StarlarkValueCell {
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        match self.get() {
            Some(v) => v.to_value().invoke(args, eval),
            None => Err(super::util::errorf(
                "internal error: Starlark value used before its module was frozen",
            )),
        }
    }
}

/// The value behind a handle: a filled cell's value, the handle's value otherwise. `None` for a
/// cell whose module has not been frozen yet.
pub fn resolve(c: StarlarkCallable) -> Option<FrozenValue> {
    let v = c.value();
    match v.downcast_ref::<StarlarkValueCell>() {
        Some(cell) => cell.get(),
        None => Some(v),
    }
}

/// True if both handles denote the same Starlark object (Java reference equality of the held
/// objects): the same cell, or the same frozen value.
pub fn same_object(a: StarlarkCallable, b: StarlarkCallable) -> bool {
    if a == b {
        return true;
    }
    match (resolve(a), resolve(b)) {
        (Some(x), Some(y)) => x.to_value().ptr_eq(y.to_value()),
        _ => false,
    }
}

/// A value captured into a model object, with the handle the model object holds.
#[derive(Debug, Clone, Copy, Trace, Allocative)]
pub struct PendingValue<'v> {
    pub value: Value<'v>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    pub handle: StarlarkCallable,
}

impl<'v> PendingValue<'v> {
    /// The handle for `value`: the value itself if already frozen, else a new cell on the
    /// module's frozen heap.
    pub fn new(value: Value<'v>, eval: &Evaluator<'v, '_, '_>) -> PendingValue<'v> {
        let handle = match value.unpack_frozen() {
            Some(fv) => StarlarkCallable::new(fv),
            None => StarlarkCallable::new(eval.frozen_heap().alloc(StarlarkValueCell {
                inner: OnceLock::new(),
            })),
        };
        PendingValue { value, handle }
    }

    /// Freezes the value and fills the cell. Returns the frozen value.
    pub fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenValue> {
        let fv = self.value.freeze(freezer)?;
        if let Some(cell) = self.handle.value().downcast_ref::<StarlarkValueCell>() {
            cell.fill(fv);
        }
        Ok(fv)
    }
}

/// Freezes every pending value (filling their cells).
pub fn freeze_all(pending: Vec<PendingValue>, freezer: &Freezer) -> FreezeResult<()> {
    for p in pending {
        p.freeze(freezer)?;
    }
    Ok(())
}
