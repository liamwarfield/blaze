//! The handle by which immutable model types hold Starlark objects (rule implementation
//! functions, computed-default callbacks, initializers, transitions, aspects, macro
//! implementations): crates/blaze_packages/DESIGN.md "Rule classes from rule()".

use std::fmt;

use starlark::docs::DocItem;
use starlark::docs::DocMember;
use starlark::values::FrozenHeapRef;
use starlark::values::FrozenValue;
use starlark::values::OwnedFrozenValue;
use starlark::values::ValueLike;

use crate::starlark_defs::value_cell::StarlarkValueCell;

/// A frozen Starlark value (usually a function) held by a [`crate::rule_class::RuleClass`],
/// [`crate::attribute::Attribute`] or macro class. Java stores the `StarlarkCallable` /
/// `StarlarkFunction` object itself.
///
/// Values are only ever handed out from frozen .bzl modules, whose heaps BZL_LOAD keeps alive
/// for the life of the process (single build per process), so the bare [`FrozenValue`] stays
/// valid. // PORT: keep-alive
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StarlarkCallable(FrozenValue);

impl StarlarkCallable {
    /// Wraps a value of a frozen module whose heap is kept alive (see the type docs).
    // PORT: keep-alive
    pub fn new(value: FrozenValue) -> Self {
        StarlarkCallable(value)
    }

    /// The frozen value, resolved through a filled `starlark_defs::value_cell` cell (the
    /// cell itself while its module is not yet frozen).
    // PORT: rule()/attr() hand out handles to cells filled when the defining module freezes.
    pub fn value(&self) -> FrozenValue {
        match self.0.to_value().downcast_ref::<StarlarkValueCell>() {
            Some(cell) => cell.get().unwrap_or(self.0),
            None => self.0,
        }
    }

    /// The handle's own value (a cell for values of a module not frozen when the handle was
    /// made).
    pub fn raw_value(&self) -> FrozenValue {
        self.0
    }

    /// The value as an [`OwnedFrozenValue`] for `blaze_starlark::eval::call_frozen`.
    pub fn to_owned_frozen_value(&self) -> OwnedFrozenValue {
        // SAFETY: the value lives on a frozen heap that BZL_LOAD keeps alive for the life of the
        // process (PORT: keep-alive), so an empty owner reference is sufficient.
        unsafe { OwnedFrozenValue::new(FrozenHeapRef::default(), self.value()) }
    }

    /// Java `StarlarkCallable.getName()` (for a `def` function, its name).
    pub fn get_name(&self) -> String {
        match self.value().to_value().function_name() {
            Some(name) => name.to_owned(),
            None => blaze_util::unported!("StarlarkCallable.getName of a non-function"),
        }
    }

    /// Java `StarlarkFunction.getParameterNames()`: all parameter names in run-time order
    /// (non-keyword-only, keyword-only, `*args`, `**kwargs`).
    // Java: net/starlark/java/eval/StarlarkFunction.java StarlarkFunction.getParameterNames
    pub fn get_parameter_names(&self) -> Vec<String> {
        match self.value().to_value().documentation() {
            DocItem::Member(DocMember::Function(f)) => {
                let p = &f.params;
                p.pos_only
                    .iter()
                    .chain(p.pos_or_named.iter())
                    .chain(p.named_only.iter())
                    .chain(p.args.iter())
                    .chain(p.kwargs.iter())
                    .map(|d| d.name.clone())
                    .collect()
            }
            _ => blaze_util::unported!("StarlarkFunction.getParameterNames of a non-function"),
        }
    }
}

impl fmt::Debug for StarlarkCallable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StarlarkCallable({})", self.0)
    }
}
