//! The Starlark `runfiles` value (Java: `analysis/Runfiles.java` is a `StarlarkValue`
//! implementing `starlarkbuildapi/RunfilesApi.java`).
//!
//! The Rust `Runfiles` is ported in `blaze_actions`; this wraps it for Starlark.

use std::fmt;

use allocative::Allocative;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_starlark::ThreadContext;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use blaze_actions::analysis::symlink_entry::SymlinkEntry;
use blaze_collect::NestedSet;
use starlark::values::{FrozenHeap, OwnedFrozenValue};

use crate::rule::depsets::{
    artifacts_to_depset, depset_value, nested_set_to_depset, strings_to_depset,
};

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// A `runfiles` object.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:72 Runfiles
// Java: com/google/devtools/build/lib/starlarkbuildapi/RunfilesApi.java:26 RunfilesApi
#[derive(Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkRunfiles(#[allocative(skip)] pub Runfiles);

starlark_simple_value!(StarlarkRunfiles);

impl StarlarkRunfiles {
    pub fn new(runfiles: Runfiles) -> StarlarkRunfiles {
        StarlarkRunfiles(runfiles)
    }

    pub fn runfiles(&self) -> &Runfiles {
        &self.0
    }

    /// The `Runfiles` of a Starlark `runfiles` value.
    pub fn runfiles_of(v: Value) -> Option<Runfiles> {
        v.downcast_ref::<StarlarkRunfiles>().map(|r| r.0.clone())
    }
}

impl fmt::Debug for StarlarkRunfiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StarlarkRunfiles")
    }
}

impl fmt::Display for StarlarkRunfiles {
    // Java: StarlarkValue.repr default for Runfiles (no repr override).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.analysis.Runfiles>"
        )
    }
}

/// The `structField` methods of `RunfilesApi`, in declaration order.
const RUNFILES_FIELDS: [&str; 4] = ["files", "symlinks", "root_symlinks", "empty_filenames"];

#[starlark_value(type = "runfiles")]
impl<'v> StarlarkValue<'v> for StarlarkRunfiles {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("runfiles", runfiles_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/analysis/Runfiles.java:228 Runfiles.getArtifactsForStarlark
            "files" => Some(depset_value(&artifacts_to_depset(self.0.get_artifacts()))),
            // Java: com/google/devtools/build/lib/analysis/Runfiles.java:238 Runfiles.getSymlinksForStarlark
            "symlinks" => Some(depset_value(&symlinks_to_depset(self.0.get_symlinks()))),
            // Java: com/google/devtools/build/lib/analysis/Runfiles.java:405 Runfiles.getRootSymlinksForStarlark
            "root_symlinks" => Some(depset_value(&symlinks_to_depset(
                self.0.get_root_symlinks(),
            ))),
            // Java: com/google/devtools/build/lib/analysis/Runfiles.java:247 Runfiles.getEmptyFilenamesForStarlark
            "empty_filenames" => Some(depset_value(&strings_to_depset(
                &self.0.get_empty_filenames_for_starlark(),
            ))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        RUNFILES_FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut v: Vec<String> = RUNFILES_FIELDS.iter().map(|s| s.to_string()).collect();
        v.extend(["merge".to_owned(), "merge_all".to_owned()]);
        v.sort();
        v
    }

    // Java: Runfiles.isImmutable (immutable and Starlark-hashable, by identity).
    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::Hash;
        (self.0.get_artifacts() as *const _ as usize).hash(hasher);
        Ok(())
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkRunfiles>()
            .is_some_and(|o| Runfiles::ptr_eq(&self.0, &o.0)))
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// `Depset.of(SymlinkEntry.class, set)`: a depset of `SymlinkEntry`s.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:180 Depset.of
fn symlinks_to_depset(set: &NestedSet<SymlinkEntry>) -> OwnedFrozenValue {
    nested_set_to_depset(
        set,
        "SymlinkEntry",
        &|e: &SymlinkEntry, heap: &FrozenHeap| {
            heap.alloc(crate::starlark::starlark_runfiles::StarlarkSymlinkEntry(
                e.clone(),
            ))
        },
    )
}

#[starlark_module]
fn runfiles_methods(builder: &mut MethodsBuilder) {
    /// Returns a new runfiles object that includes all the contents of this one and the
    /// argument.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:835 Runfiles.merge
    fn merge<'v>(
        this: &StarlarkRunfiles,
        #[starlark(require = pos)] other: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkRunfiles> {
        let o = StarlarkRunfiles::runfiles_of(other).ok_or_else(|| {
            errorf(format!(
                "in call to merge(), parameter 'other' got value of type '{}', want 'runfiles'",
                other.get_type()
            ))
        })?;
        let limit = ThreadContext::get(eval)?.semantics.nested_set_depth_limit;
        this.0
            .merge(&o, limit)
            .map(StarlarkRunfiles)
            .map_err(|e| errorf(e.to_string()))
    }

    /// Returns a new runfiles object that includes all the contents of this one and of the
    /// runfiles objects in the provided sequence.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:848 Runfiles.mergeAll
    fn merge_all<'v>(
        this: &StarlarkRunfiles,
        #[starlark(require = pos)] other: UnpackListOrTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkRunfiles> {
        // Java: Sequence.cast(sequence, Runfiles.class, "param").
        let mut sequence = Vec::with_capacity(other.items.len());
        for (i, v) in other.items.iter().enumerate() {
            match StarlarkRunfiles::runfiles_of(*v) {
                Some(r) => sequence.push(r),
                None => {
                    return Err(errorf(format!(
                        "at index {i} of param, got element of type {}, want runfiles",
                        v.get_type()
                    )));
                }
            }
        }
        let limit = ThreadContext::get(eval)?.semantics.nested_set_depth_limit;
        this.0
            .merge_all(&sequence, limit)
            .map(StarlarkRunfiles)
            .map_err(|e| errorf(e.to_string()))
    }
}
