//! `ModuleExtension`: a module extension object, created by `module_extension()`.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtension.java. The Starlark value is
//! `crate::starlark::repository_module::ModuleExtensionValue`; [`ModuleExtension`] is the
//! immutable Rust data read from a frozen, exported value.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_starlark::Location;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use super::tag_class::TagClass;

/// A module extension object, which can be used to perform arbitrary logic in order to create
/// repos.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtension.java:30 ModuleExtension
pub struct ModuleExtension {
    /// PORT: keep-alive (the frozen heap of the defining .bzl).
    pub implementation: OwnedFrozenValue,
    pub tag_classes: IndexMap<String, Arc<TagClass>>,
    pub doc: Option<String>,
    pub defining_bzl_file_label: Label,
    pub location: Location,
    pub env_variables: Vec<String>,
    pub os_dependent: bool,
    pub arch_dependent: bool,
    pub facts_version: i32,
    /// The `BazelModuleContext` of the .bzl that exported the extension.
    pub module_context: Option<Arc<BazelModuleContext>>,
}

impl std::fmt::Debug for ModuleExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleExtension")
            .field("defining_bzl_file_label", &self.defining_bzl_file_label)
            .field("tag_classes", &self.tag_classes.keys().collect::<Vec<_>>())
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}
