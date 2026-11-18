//! A local wrapper that lets option fields hold types from other crates.
//!
//! PORT: Rust's orphan rule forbids implementing `blaze_options`' `OptionScalar`/`FieldType` for
//! `blaze_vfs`/`blaze_cmdline` types in this crate, and `blaze_options` does not depend on those
//! crates. Option fields whose Java type is `PathFragment`, `PackageIdentifier`, `Label` or
//! `DigestHashFunction` therefore hold `Foreign<T>`, which derefs to `T`.

use std::fmt;
use std::ops::Deref;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_options::{OptionScalar, option_field_type};
use blaze_vfs::{DigestHashFunction, PathFragment};

/// Holds a value of a type declared in another crate; see the module docs.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Foreign<T>(pub T);

impl<T> Deref for Foreign<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for Foreign<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java PathFragment.toString
impl OptionScalar for Foreign<PathFragment> {
    fn java_to_string(&self) -> String {
        self.0.get_path_string().to_string()
    }
}
option_field_type!(
    Foreign<PathFragment>,
    "com.google.devtools.build.lib.vfs.PathFragment",
    raw = Foreign(PathFragment::empty_fragment())
);

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:267 PackageIdentifier.toString
impl OptionScalar for Foreign<PackageIdentifier> {
    fn java_to_string(&self) -> String {
        if self.0.get_repository().is_main() {
            return self.0.get_package_fragment().get_path_string().to_string();
        }
        self.0.get_canonical_form()
    }
}
option_field_type!(
    Foreign<PackageIdentifier>,
    "com.google.devtools.build.lib.cmdline.PackageIdentifier",
    raw = Foreign(PackageIdentifier::create_in_main_repo_str(""))
);

// Java: com/google/devtools/build/lib/cmdline/Label.java:479 Label.toString
impl OptionScalar for Foreign<Label> {
    fn java_to_string(&self) -> String {
        self.0.get_canonical_form()
    }
}

// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:174 DigestHashFunction.toString
impl OptionScalar for Foreign<DigestHashFunction> {
    fn java_to_string(&self) -> String {
        self.0.name().to_string()
    }
}
option_field_type!(
    Foreign<DigestHashFunction>,
    "com.google.devtools.build.lib.vfs.DigestHashFunction",
    raw = Foreign(DigestHashFunction::Sha256)
);
