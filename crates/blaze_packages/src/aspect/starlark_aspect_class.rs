//! Port of `packages/StarlarkAspectClass.java`.

use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use blaze_cmdline::Label;

/// The [`super::AspectClass`] of aspects defined in Starlark (Java `StarlarkAspectClass`):
/// the defining .bzl file and the name the aspect is exported as.
///
/// PORT: Java keys the class by the `BzlLoadValue.Key` of the .bzl; Blaze keeps its label.
/// Java creates the key with `BzlLoadValue.keyForBuild` everywhere aspects are named (exported
/// aspects of builtins .bzl files are not applied as aspects), so the label identifies it.
// Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:26 StarlarkAspectClass
#[derive(Clone)]
pub struct StarlarkAspectClass(Arc<Inner>);

struct Inner {
    extension_label: Label,
    exported_name: String,
    name: String,
}

impl StarlarkAspectClass {
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:31 StarlarkAspectClass.<init>
    pub fn new(extension_label: Label, exported_name: &str) -> StarlarkAspectClass {
        let name = format!("{extension_label}%{exported_name}");
        StarlarkAspectClass(Arc::new(Inner {
            extension_label,
            exported_name: exported_name.to_owned(),
            name,
        }))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:41 StarlarkAspectClass.getExtensionLabel
    pub fn get_extension_label(&self) -> &Label {
        &self.0.extension_label
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:45 StarlarkAspectClass.getExportedName
    pub fn get_exported_name(&self) -> &str {
        &self.0.exported_name
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:50 StarlarkAspectClass.getName
    pub fn get_name(&self) -> &str {
        &self.0.name
    }

    /// Parses `--exec_aspects`-style names (`//foo/bar:baz.bzl%my_aspect`).
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:78 StarlarkAspectClass.getAspectClassFromName
    pub fn get_aspect_class_from_name(
        aspect: &str,
    ) -> Result<StarlarkAspectClass, AspectClassCreationException> {
        if let Some(delimiter_position) = aspect.find('%') {
            let bzl_file_load_like_string = &aspect[..delimiter_position];
            if !bzl_file_load_like_string.starts_with("//")
                && !bzl_file_load_like_string.starts_with('@')
            {
                Err(AspectClassCreationException(format!(
                    "--exec_aspects must be specified with absolute labels, e.g. \
                     //foo/bar:baz.bzl%my_aspect, @repo//foo/bar:baz%my_aspect, or \
                     /foo/bar:baz.bzl%my_aspect. Found: {aspect}"
                )))
            } else if !bzl_file_load_like_string.ends_with(".bzl") {
                Err(AspectClassCreationException(format!(
                    "--exec_aspects files must end with .bzl. Found: {aspect}"
                )))
            } else {
                match Label::parse_canonical(bzl_file_load_like_string) {
                    Ok(starlark_file_label) => {
                        let starlark_function_name = &aspect[delimiter_position + 1..];
                        Ok(StarlarkAspectClass::new(
                            starlark_file_label,
                            starlark_function_name,
                        ))
                    }
                    Err(e) => Err(AspectClassCreationException(format!(
                        "Invalid aspect '{aspect}': {}",
                        e.get_message()
                    ))),
                }
            }
        } else {
            Err(AspectClassCreationException(format!(
                "--exec_aspects must include the aspect name, preceded by '%', e.g. \
                 //foo/bar:baz.bzl%my_aspect, @repo//foo/bar:baz%my_aspect, or \
                 /foo/bar:baz.bzl%my_aspect. Found: {aspect}"
            )))
        }
    }
}

impl PartialEq for StarlarkAspectClass {
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:55 StarlarkAspectClass.equals
    fn eq(&self, other: &StarlarkAspectClass) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.extension_label == other.0.extension_label
                && self.0.exported_name == other.0.exported_name)
    }
}

impl Eq for StarlarkAspectClass {}

impl Hash for StarlarkAspectClass {
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:69 StarlarkAspectClass.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.extension_label.hash(state);
        self.0.exported_name.hash(state);
    }
}

impl fmt::Display for StarlarkAspectClass {
    // Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:74 StarlarkAspectClass.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

impl fmt::Debug for StarlarkAspectClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

/// A problem creating a [`StarlarkAspectClass`] (Java
/// `StarlarkAspectClass.AspectClassCreationException`).
// Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:116 StarlarkAspectClass.AspectClassCreationException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AspectClassCreationException(pub String);
