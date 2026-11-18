//! [`Foreign<T>`]: the option field type for values whose type lives in another crate.
//!
//! PORT: Java option fields hold `Label`, `PackageIdentifier`, `PathFragment` and
//! `DigestHashFunction` directly. In Rust, [`OptionScalar`] and [`crate::FieldType`] are traits of
//! this crate and those types belong to `blaze_cmdline` / `blaze_vfs`; the orphan rule lets only
//! this crate (or the type's own crate) implement one for the other. Neither `blaze_cmdline` nor
//! `blaze_vfs` depends on `blaze_options`, so the implementations live here, on a single
//! transparent wrapper, rather than on the bare types: option fields of those Java types are
//! declared as `Foreign<Label>`, `Option<Foreign<PathFragment>>`, `Vec<Foreign<Label>>`, ...
//!
//! `Foreign<T>` dereferences to `T`, and its `Debug`/`Display`/`Eq`/`Ord`/`Hash` are `T`'s.
//! Converters build values with `Value::new(Foreign(t))`.
//!
//! Only the types listed below are supported. A crate further downstream cannot add its own
//! `impl OptionScalar for Foreign<ItsType>` (the orphan rule forbids that as well); such a type
//! needs a local newtype implementing [`OptionScalar`] in that crate.

use std::fmt;
use std::ops::Deref;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_vfs::{DigestHashFunction, PathFragment};

use crate::value::OptionScalar;

/// A value of a type declared in another crate, usable as an option field type; see the module
/// docs.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Foreign<T>(pub T);

impl<T> Foreign<T> {
    /// The wrapped value.
    pub fn get(&self) -> &T {
        &self.0
    }

    /// Unwraps the value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Foreign<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> From<T> for Foreign<T> {
    fn from(t: T) -> Self {
        Foreign(t)
    }
}

impl<T: fmt::Debug> fmt::Debug for Foreign<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<T: fmt::Display> fmt::Display for Foreign<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// Java: com/google/devtools/build/lib/cmdline/Label.java:479 Label.toString
impl OptionScalar for Foreign<Label> {
    fn java_to_string(&self) -> String {
        self.0.get_canonical_form()
    }
}
crate::option_field_type!(
    Foreign<Label>,
    "com.google.devtools.build.lib.cmdline.Label",
    // Placeholder until the parser sets the field (Java holds null there).
    raw = Foreign(Label::parse_canonical_unchecked("//:__unset__"))
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
crate::option_field_type!(
    Foreign<PackageIdentifier>,
    "com.google.devtools.build.lib.cmdline.PackageIdentifier",
    raw = Foreign(PackageIdentifier::create_in_main_repo_str(""))
);

// Java: com/google/devtools/build/lib/vfs/PathFragment.java PathFragment.toString
impl OptionScalar for Foreign<PathFragment> {
    fn java_to_string(&self) -> String {
        self.0.get_path_string().to_string()
    }
}
crate::option_field_type!(
    Foreign<PathFragment>,
    "com.google.devtools.build.lib.vfs.PathFragment",
    raw = Foreign(PathFragment::empty_fragment())
);

// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:174 DigestHashFunction.toString
impl OptionScalar for Foreign<DigestHashFunction> {
    fn java_to_string(&self) -> String {
        self.0.name().to_string()
    }
}
crate::option_field_type!(
    Foreign<DigestHashFunction>,
    "com.google.devtools.build.lib.vfs.DigestHashFunction",
    raw = Foreign(DigestHashFunction::Sha256)
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_to_string_matches_java_to_string() {
        assert_eq!(
            Foreign(Label::parse_canonical_unchecked("//a:b")).java_to_string(),
            "//a:b"
        );
        assert_eq!(
            Foreign(PathFragment::create("a//b/")).java_to_string(),
            "a/b"
        );
        assert_eq!(
            Foreign(PackageIdentifier::create_in_main_repo_str("x/y")).java_to_string(),
            "x/y"
        );
    }
}
