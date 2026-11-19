//! Port of `packages/AspectDescriptor.java`.

use std::fmt;
use std::sync::Arc;

use super::AspectClass;
use super::AspectParameters;

/// A pair of [`AspectClass`] and [`AspectParameters`], used for dependency resolution and in
/// Skyframe keys (Java `AspectDescriptor`).
///
/// PORT: Java interns descriptors; equality is structural either way.
// Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:37 AspectDescriptor
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AspectDescriptor(Arc<Inner>);

#[derive(PartialEq, Eq, Hash)]
struct Inner {
    aspect_class: AspectClass,
    aspect_parameters: AspectParameters,
}

impl AspectDescriptor {
    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:42 AspectDescriptor.of
    pub fn of(aspect_class: AspectClass, aspect_parameters: AspectParameters) -> AspectDescriptor {
        AspectDescriptor(Arc::new(Inner {
            aspect_class,
            aspect_parameters,
        }))
    }

    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:54 AspectDescriptor.getAspectClass
    pub fn get_aspect_class(&self) -> &AspectClass {
        &self.0.aspect_class
    }

    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:58 AspectDescriptor.getParameters
    pub fn get_parameters(&self) -> &AspectParameters {
        &self.0.aspect_parameters
    }

    /// Creates a presentable description of this aspect, available to Starlark via
    /// "Target.aspects".
    ///
    /// The description is designed to be unique for each aspect descriptor, but not to be
    /// parseable.
    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:92 AspectDescriptor.getDescription
    pub fn get_description(&self) -> String {
        if self.0.aspect_parameters.is_empty() {
            return self.0.aspect_class.get_name().to_owned();
        }

        let mut builder = String::from(self.0.aspect_class.get_name());
        builder.push('[');
        let mut first = true;
        for (key, value) in self.0.aspect_parameters.entries() {
            if !first {
                builder.push(',');
            } else {
                first = false;
            }
            builder.push_str(key);
            builder.push_str("=\"");
            builder.push_str(&escape_double_quotes_and_backslashes(value));
            builder.push('"');
        }
        builder.push(']');
        builder
    }
}

/// Protobuf `TextFormat.escapeDoubleQuotesAndBackslashes`.
// Java: com/google/protobuf/TextFormat.java TextFormat.escapeDoubleQuotesAndBackslashes
fn escape_double_quotes_and_backslashes(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

impl fmt::Display for AspectDescriptor {
    // Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:82 AspectDescriptor.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_description())
    }
}

impl fmt::Debug for AspectDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_description())
    }
}
