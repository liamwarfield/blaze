//! Port of `analysis/DormantDependency.java`.

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark_map::StarlarkHasher;

/// Not an actual dependency, but the possibility of one.
///
/// Dormant attributes result in an instance of this object for each possible dependency edge.
/// It can then be passed up the dependency graph and turned into an actual dependency
/// ("materialized") by rules in the reverse transitive closure.
// Java: com/google/devtools/build/lib/analysis/DormantDependency.java:37 DormantDependency
#[derive(Clone, Debug, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct DormantDependency {
    #[allocative(skip)]
    label: Label,
}

starlark_simple_value!(DormantDependency);

impl DormantDependency {
    // Java: com/google/devtools/build/lib/analysis/DormantDependency.java:36 DormantDependency.<init>
    pub fn new(label: Label) -> DormantDependency {
        DormantDependency { label }
    }

    // Java: com/google/devtools/build/lib/analysis/DormantDependency.java:52 DormantDependency.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    /// The dormant dependency a Starlark value is, if any.
    pub fn of(v: Value<'_>) -> Option<&DormantDependency> {
        v.downcast_ref::<DormantDependency>()
    }
}

impl fmt::Display for DormantDependency {
    // Java: com/google/devtools/build/lib/analysis/DormantDependency.java:45 DormantDependency.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<dormant dependency label='{}'>", self.label)
    }
}

// Java: StarlarkValue of a class without @StarlarkBuiltin: Starlark.type is the simple class
// name.
#[starlark_value(type = "DormantDependency")]
impl<'v> StarlarkValue<'v> for DormantDependency {
    // Java: com/google/devtools/build/lib/analysis/DormantDependency.java:52 DormantDependency.getLabel
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "label" => Some(heap.alloc(LabelValue(self.label.clone()))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "label"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["label".to_owned()]
    }

    // Java: a record: equals compares the label.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(DormantDependency::of(other).is_some_and(|o| o == self))
    }

    // Java: com/google/devtools/build/lib/analysis/DormantDependency.java:57 DormantDependency.isImmutable
    // (an immutable value is hashable: StarlarkValue.isHashable; the record's hashCode is the
    // label's).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.label.hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}
