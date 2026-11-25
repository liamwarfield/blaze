//! `rules/java/BootClassPathInfo.java`: information about the system APIs for a Java
//! compilation (a wrapped instance of rules_java's `BootClassPathInfo`).

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_vfs::PathFragment;
use starlark::values::{Heap, Value};

use super::starlark_info_wrapper::StarlarkInfoWrapper;

/// Information about the system APIs for a Java compilation.
///
/// PORT: Java's `EMPTY` instance (an anonymous subclass with no underlying struct) is
/// `underlying == None`.
// Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:29 BootClassPathInfo
#[derive(Clone, Copy)]
pub struct BootClassPathInfo<'v> {
    underlying: Option<StarlarkInfoWrapper<'v>>,
}

impl<'v> BootClassPathInfo<'v> {
    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:59 BootClassPathInfo.empty
    pub fn empty() -> BootClassPathInfo<'v> {
        BootClassPathInfo { underlying: None }
    }

    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:67 BootClassPathInfo.wrap
    pub fn wrap(info: Value<'v>, heap: Heap<'v>) -> BootClassPathInfo<'v> {
        BootClassPathInfo {
            underlying: Some(StarlarkInfoWrapper::new(info, heap)),
        }
    }

    /// The jar files containing classes for system APIs, i.e. a Java <= 8 bootclasspath.
    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:72 BootClassPathInfo.bootclasspath
    pub fn bootclasspath(&self) -> starlark::Result<NestedSet<Artifact>> {
        match &self.underlying {
            None => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(w) => w.get_underlying_nested_set_artifacts("bootclasspath"),
        }
    }

    /// The jar files containing extra classes for system APIs that should not be put in the
    /// system image to support split-package compilation scenarios.
    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:80 BootClassPathInfo.auxiliary
    pub fn auxiliary(&self) -> starlark::Result<NestedSet<Artifact>> {
        match &self.underlying {
            None => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(w) => w.get_underlying_nested_set_artifacts("_auxiliary"),
        }
    }

    /// Contents of the directory that is passed to the javac >= 9 `--system` flag.
    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:85 BootClassPathInfo.systemInputs
    pub fn system_inputs(&self) -> starlark::Result<NestedSet<Artifact>> {
        match &self.underlying {
            None => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(w) => w.get_underlying_nested_set_artifacts("_system_inputs"),
        }
    }

    /// An argument to the javac >= 9 `--system` flag.
    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:90 BootClassPathInfo.systemPath
    pub fn system_path(&self) -> starlark::Result<Option<PathFragment>> {
        match &self.underlying {
            None => Ok(None),
            Some(w) => Ok(w
                .get_underlying_string("_system_path")?
                .map(|s| PathFragment::create(&s))),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/BootClassPathInfo.java:95 BootClassPathInfo.isEmpty
    pub fn is_empty(&self) -> starlark::Result<bool> {
        if self.underlying.is_none() {
            return Ok(true);
        }
        Ok(self.bootclasspath()?.is_empty()
            && self.auxiliary()?.is_empty()
            && self.system_inputs()?.is_empty()
            && self.system_path()?.is_none())
    }
}
