//! The Starlark `runfiles` value (Java `analysis/Runfiles.java` as a `RunfilesApi`
//! StarlarkValue, `starlarkbuildapi/RunfilesApi.java`) over
//! `blaze_actions::analysis::runfiles::Runfiles`, and the `SymlinkEntry` value
//! (`analysis/SymlinkEntry.java`, `starlarkbuildapi/SymlinkEntryApi.java`) of its symlink depsets.
//!
//! Other crates (py_builtins) wrap and unwrap runfiles with [`runfiles_to_value`] and
//! [`runfiles_from_value`].

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_actions::analysis::symlink_entry::SymlinkEntry;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

pub use crate::providers::starlark_runfiles::StarlarkRunfiles;

/// Allocates `runfiles` as a Starlark value.
// PORT: the Starlark `runfiles` value is R's `crate::providers::starlark_runfiles` (DefaultInfo
// unwraps it); this module only adds the `SymlinkEntry` value its symlink depsets hold.
pub fn runfiles_to_value<'v>(heap: Heap<'v>, runfiles: Runfiles) -> Value<'v> {
    heap.alloc(StarlarkRunfiles::new(runfiles))
}

/// Java `x instanceof Runfiles ? (Runfiles) x : null`.
pub fn runfiles_from_value(v: Value) -> Option<Runfiles> {
    StarlarkRunfiles::runfiles_of(v)
}

// -------------------------------------------------------------------------------------------
// SymlinkEntry

/// A runfiles symlink entry as a Starlark value (type `SymlinkEntry`).
// Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:30 SymlinkEntry
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkSymlinkEntry(#[allocative(skip)] pub SymlinkEntry);

starlark_simple_value!(StarlarkSymlinkEntry);

impl fmt::Display for StarlarkSymlinkEntry {
    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java SymlinkEntry.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.repr())
    }
}

#[starlark_value(type = "SymlinkEntry")]
impl<'v> StarlarkValue<'v> for StarlarkSymlinkEntry {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("SymlinkEntry", symlink_entry_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.0.repr());
    }

    // Java: SymlinkEntry is an AutoValue-like record: equal by path and artifact.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkSymlinkEntry>()
            .is_some_and(|o| {
                o.0.get_path() == self.0.get_path() && o.0.get_artifact() == self.0.get_artifact()
            }))
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.get_path().hash(hasher);
        self.0.get_artifact().hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn symlink_entry_methods(builder: &mut MethodsBuilder) {
    /// The requested path of the symlink in the runfiles tree.
    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java SymlinkEntry.getPathString
    #[starlark(attribute)]
    fn path(this: &StarlarkSymlinkEntry) -> starlark::Result<String> {
        Ok(this.0.get_path_string().to_owned())
    }

    /// Target file of the symlink.
    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java SymlinkEntry.getArtifact
    #[starlark(attribute)]
    fn target_file(this: &StarlarkSymlinkEntry) -> starlark::Result<StarlarkFile> {
        Ok(StarlarkFile::new(this.0.get_artifact().clone()))
    }
}

/// Java `x instanceof SymlinkEntry`.
pub fn symlink_entry_from_value(v: Value) -> Option<SymlinkEntry> {
    v.downcast_ref::<StarlarkSymlinkEntry>()
        .map(|e| e.0.clone())
}
