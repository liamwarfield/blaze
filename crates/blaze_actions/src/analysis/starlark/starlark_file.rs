//! The Starlark `File` value (`lib/starlarkbuildapi/FileApi.java`, implemented by
//! `lib/actions/Artifact.java`) and the Starlark `root` value (`lib/starlarkbuildapi/
//! FileRootApi.java`, implemented by `ArtifactRoot` and `PathMapper.MappedArtifactRoot`).
//!
//! In Java an `Artifact` *is* a Starlark value. Here [`StarlarkFile`] wraps the shared
//! [`Artifact`] handle; equality and hashing are the artifact's (Java `Artifact.equals` /
//! `hashCode`), and the getters are the `@StarlarkMethod(structField = true)` methods of FileApi.

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;

use crate::actions::artifact::Artifact;
use crate::actions::path_mapper::{FileRoot, NOOP, PathMapper};

fn eval_err(e: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{e}"))
}

// PORT: Java stores a path mapper in the thread's StarlarkSemantics (`PathMapper.storeIn`) while
// a `map_each` callback of a custom command line runs with a non-trivial path mapper; everywhere
// else it is `PathMapper.NOOP`. Starlark semantics here carry no such entry, so the mapper is
// kept in a thread-local for the duration of the callback ([`with_stored_path_mapper`]); the
// callback is evaluated on the calling thread.
thread_local! {
    static STORED_PATH_MAPPER: std::cell::Cell<Option<*const dyn PathMapper>> =
        const { std::cell::Cell::new(None) };
}

/// Runs `f` with `path_mapper` as the mapper the `File` getters apply
/// (`StarlarkThread.createTransient(mu, pathMapper.storeIn(starlarkSemantics))`).
// Java: com/google/devtools/build/lib/actions/PathMapper.java:65 PathMapper.storeIn
pub fn with_stored_path_mapper<R>(path_mapper: &dyn PathMapper, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<*const dyn PathMapper>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STORED_PATH_MAPPER.with(|c| c.set(self.0));
        }
    }
    // SAFETY (of the stored pointer's later use): `with_loaded_path_mapper` dereferences it only while
    // this call runs; `Restore` resets the thread-local before `path_mapper`'s borrow ends, even
    // when `f` panics.
    let ptr: *const (dyn PathMapper + '_) = path_mapper;
    // Only the lifetime bound of the trait object changes.
    let ptr: *const dyn PathMapper = unsafe { std::mem::transmute(ptr) };
    let _restore = Restore(STORED_PATH_MAPPER.with(|c| c.replace(Some(ptr))));
    f()
}

/// `PathMapper.loadFrom(semantics)`: the mapper stored for the running callback, else NOOP.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:47 PathMapper.loadFrom
fn with_loaded_path_mapper<R>(f: impl FnOnce(&dyn PathMapper) -> R) -> R {
    match STORED_PATH_MAPPER.with(|c| c.get()) {
        // SAFETY: see `with_stored_path_mapper`.
        Some(ptr) => f(unsafe { &*ptr }),
        None => f(&NOOP),
    }
}

/// A `File` (an [`Artifact`]) as a Starlark value.
// Java: com/google/devtools/build/lib/starlarkbuildapi/FileApi.java:28 FileApi
// Java: com/google/devtools/build/lib/actions/Artifact.java:119 Artifact
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkFile(#[allocative(skip)] pub Artifact);

starlark_simple_value!(StarlarkFile);

impl StarlarkFile {
    pub fn new(artifact: Artifact) -> StarlarkFile {
        StarlarkFile(artifact)
    }

    pub fn artifact(&self) -> &Artifact {
        &self.0
    }
}

/// Returns the artifact of a Starlark `File` value, if `v` is one.
pub fn artifact_from_value(v: Value) -> Option<Artifact> {
    v.downcast_ref::<StarlarkFile>().map(|f| f.0.clone())
}

impl fmt::Display for StarlarkFile {
    // Java: com/google/devtools/build/lib/actions/Artifact.java:1256 Artifact.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.repr())
    }
}

#[starlark_value(type = "File")]
impl<'v> StarlarkValue<'v> for StarlarkFile {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("File", file_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:1256 Artifact.repr
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.0.repr());
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:677 Artifact.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkFile>()
            .is_some_and(|o| o.0 == self.0))
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:701 Artifact.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:197 Artifact.compareTo
    fn compare(&self, other: Value<'v>) -> starlark::Result<std::cmp::Ordering> {
        match other.downcast_ref::<StarlarkFile>() {
            Some(o) => Ok(self.0.compare_to(&o.0)),
            None => Err(eval_err(format!(
                "unsupported comparison: File <=> {}",
                other.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/actions/Artifact.java:1251 Artifact.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

// Java: com/google/devtools/build/lib/actions/Artifact.java:388 Artifact.getDirname
fn get_dirname(exec_path: &blaze_vfs::PathFragment) -> String {
    match exec_path.get_parent_directory() {
        None => "/".to_string(),
        Some(parent) => parent.get_safe_path_string().to_string(),
    }
}

/// `Artifact.getDirnameForStarlark`.
// Java: com/google/devtools/build/lib/actions/Artifact.java:394 Artifact.getDirnameForStarlark
pub fn get_dirname_for_starlark(artifact: &Artifact) -> String {
    with_loaded_path_mapper(|m| get_dirname(&m.map(artifact.get_exec_path())))
}

/// `Artifact.getExecPathStringForStarlark`.
// Java: com/google/devtools/build/lib/actions/Artifact.java:470 Artifact.getExecPathStringForStarlark
pub fn get_exec_path_string_for_starlark(artifact: &Artifact) -> String {
    with_loaded_path_mapper(|m| m.get_mapped_exec_path_string(artifact))
}

/// `Artifact.getRootForStarlark`.
// Java: com/google/devtools/build/lib/actions/Artifact.java:460 Artifact.getRootForStarlark
pub fn get_root_for_starlark(artifact: &Artifact) -> FileRoot {
    with_loaded_path_mapper(|m| m.map_root(artifact))
}

#[starlark_module]
fn file_methods(builder: &mut MethodsBuilder) {
    /// Java: FileApi.getDirnameForStarlark (Artifact.java:394)
    #[starlark(attribute)]
    fn dirname(this: &StarlarkFile) -> anyhow::Result<String> {
        Ok(get_dirname_for_starlark(&this.0))
    }

    /// Java: FileApi.getFilename (Artifact.java:400)
    #[starlark(attribute)]
    fn basename(this: &StarlarkFile) -> anyhow::Result<String> {
        Ok(this.0.get_filename().to_owned())
    }

    /// Java: FileApi.getExtension (Artifact.java:405)
    #[starlark(attribute)]
    fn extension(this: &StarlarkFile) -> anyhow::Result<String> {
        Ok(this.0.get_extension().to_owned())
    }

    /// Java: FileApi.getOwnerLabel (Artifact.java:294, 798); `allowReturnNones = true`.
    #[starlark(attribute)]
    fn owner(this: &StarlarkFile) -> anyhow::Result<NoneOr<LabelValue>> {
        Ok(match this.0.get_owner_label() {
            Some(label) => NoneOr::Other(LabelValue(label)),
            None => NoneOr::None,
        })
    }

    /// Java: FileApi.getRootForStarlark (Artifact.java:460)
    #[starlark(attribute)]
    fn root(this: &StarlarkFile) -> anyhow::Result<StarlarkFileRoot> {
        Ok(StarlarkFileRoot(get_root_for_starlark(&this.0)))
    }

    /// Java: FileApi.isSourceArtifact (Artifact.java:563)
    #[starlark(attribute)]
    fn is_source(this: &StarlarkFile) -> anyhow::Result<bool> {
        Ok(this.0.is_source_artifact())
    }

    /// Java: FileApi.isDirectory (Artifact.java:618)
    #[starlark(attribute)]
    fn is_directory(this: &StarlarkFile) -> anyhow::Result<bool> {
        Ok(this.0.is_directory())
    }

    /// Java: FileApi.isSymlink (Artifact.java:535, 877)
    #[starlark(attribute)]
    fn is_symlink(this: &StarlarkFile) -> anyhow::Result<bool> {
        Ok(this.0.is_symlink())
    }

    /// Java: FileApi.getRunfilesPathString (Artifact.java:665)
    #[starlark(attribute)]
    fn short_path(this: &StarlarkFile) -> anyhow::Result<String> {
        Ok(this.0.get_runfiles_path_string())
    }

    /// Java: FileApi.getExecPathStringForStarlark (Artifact.java:470)
    #[starlark(attribute)]
    fn path(this: &StarlarkFile) -> anyhow::Result<String> {
        Ok(get_exec_path_string_for_starlark(&this.0))
    }

    /// Java: FileApi.getTreeRelativePathString (Artifact.java:549, 1148)
    #[starlark(attribute)]
    fn tree_relative_path(this: &StarlarkFile) -> starlark::Result<String> {
        this.0.get_tree_relative_path_string().map_err(eval_err)
    }
}

/// A file root (`ArtifactRoot` or `PathMapper.MappedArtifactRoot`) as a Starlark value (type
/// `root`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/FileRootApi.java:32 FileRootApi
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkFileRoot(#[allocative(skip)] pub FileRoot);

starlark_simple_value!(StarlarkFileRoot);

impl StarlarkFileRoot {
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:285 ArtifactRoot.repr
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:269 MappedArtifactRoot.repr
    fn repr(&self) -> &'static str {
        match &self.0 {
            FileRoot::Artifact(r) => r.repr(),
            FileRoot::Mapped(m) => m.repr(),
        }
    }
}

impl fmt::Display for StarlarkFileRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.repr())
    }
}

#[starlark_value(type = "root")]
impl<'v> StarlarkValue<'v> for StarlarkFileRoot {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("root", file_root_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(self.repr());
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:266 ArtifactRoot.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkFileRoot>()
            .is_some_and(|o| o.0 == self.0))
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:256 ArtifactRoot.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:251 ArtifactRoot.compareTo
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:242 MappedArtifactRoot.compareTo
    fn compare(&self, other: Value<'v>) -> starlark::Result<std::cmp::Ordering> {
        match (other.downcast_ref::<StarlarkFileRoot>(), &self.0) {
            (Some(StarlarkFileRoot(FileRoot::Artifact(o))), FileRoot::Artifact(s)) => {
                Ok(s.compare_to(o))
            }
            (Some(StarlarkFileRoot(FileRoot::Mapped(o))), FileRoot::Mapped(s)) => Ok(s.cmp(o)),
            _ => Err(eval_err(format!(
                "unsupported comparison: root <=> {}",
                other.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java ArtifactRoot.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn file_root_methods(builder: &mut MethodsBuilder) {
    /// Java: FileRootApi.getExecPathString (ArtifactRoot.java, PathMapper.java:241)
    #[starlark(attribute)]
    fn path(this: &StarlarkFileRoot) -> anyhow::Result<String> {
        Ok(this.0.get_exec_path_string())
    }
}
