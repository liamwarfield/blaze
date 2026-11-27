//! Port of `starlarkdocextract/LabelRenderer.java`.

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_starlark::label::LabelValue;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value, ValueLike};

/// A renderer of labels (and of `repr()`s of Starlark values containing labels) in API
/// documentation.
// Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:29 LabelRenderer
#[derive(Debug, Clone)]
pub struct LabelRenderer {
    repository_mapping: RepositoryMapping,
    main_repo_name: Option<String>,
}

impl LabelRenderer {
    /// A LabelRenderer which always uses `Label.getShorthandDisplayForm` for rendering.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:34 LabelRenderer.DEFAULT
    pub fn default_renderer() -> LabelRenderer {
        LabelRenderer::new(blaze_cmdline::repository_mapping::EMPTY.clone(), None)
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:38 LabelRenderer.<init>
    pub fn new(repository_mapping: RepositoryMapping, main_repo_name: Option<String>) -> Self {
        LabelRenderer {
            repository_mapping,
            main_repo_name,
        }
    }

    /// Renders a label as an unquoted string via `Label.getShorthandDisplayForm`, further adding
    /// an explicit repo component for labels in the main repo if `main_repo_name` was provided.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:48 LabelRenderer.render
    pub fn render(&self, label: &Label) -> String {
        self.render_with(label, /* shorthand= */ true)
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:54 LabelRenderer.render
    fn render_with(&self, label: &Label, shorthand: bool) -> String {
        let label_string = if shorthand {
            label.get_shorthand_display_form(Some(&self.repository_mapping))
        } else {
            label.get_display_form(Some(&self.repository_mapping))
        };
        match &self.main_repo_name {
            None => label_string,
            Some(_) if label_string.starts_with('@') => label_string,
            Some(main_repo_name) => {
                // label.getShorthandDisplayForm omits the repo name part for labels in the main
                // repo regardless of what repositoryMapping says. Therefore, if we want to rename
                // the main repo in labels in emitted docs, we have to do so manually.
                if shorthand && label_string == format!("//:{main_repo_name}") {
                    // Special case: the shorthand form of "@foo//:foo" is "@foo".
                    return format!("@{main_repo_name}");
                }
                format!("@{main_repo_name}{label_string}")
            }
        }
    }

    /// Renders the `repr()` of a Starlark value as a string, with any embedded label values first
    /// converted to string values via [`Self::render`].
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:76 LabelRenderer.reprWithoutLabelConstructor
    pub fn repr_without_label_constructor(&self, value: Value<'_>) -> String {
        let mut out = String::new();
        repr_with(value, &mut out, &|label, out| {
            out.push_str(&string_repr(&self.render(label)));
        });
        out
    }

    /// Renders the `repr()` of a Starlark value as a string, with the argument to the `Label`
    /// constructor for any embedded label values produced via `Label.getDisplayForm`, further
    /// adding an explicit repo component for labels in the main repo if `main_repo_name` was
    /// provided.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:98 LabelRenderer.repr
    pub fn repr(&self, value: Value<'_>) -> String {
        let mut out = String::new();
        repr_with(value, &mut out, &|label, out| {
            out.push_str("Label(");
            // For consistency with Starlark.repr(label), we use label.getDisplayForm() instead
            // of the shorthand form.
            out.push_str(&string_repr(
                &self.render_with(label, /* shorthand= */ false),
            ));
            out.push(')');
        });
        out
    }
}

/// `Printer.repr` of a string.
fn string_repr(s: &str) -> String {
    Heap::temp(|heap| heap.alloc_str(s).to_value().to_repr())
}

/// `Printer.repr(o)` with `repr` overridden for labels: the overriding printer is used for the
/// elements of the containers Java's `Printer` prints itself.
///
/// PORT: Java's override also reaches the elements of any value whose `repr(Printer)` prints
/// them through the printer (a struct, a depset, ...); here only lists, tuples and dicts are
/// traversed, other values print with their own `repr`.
// Java: net/starlark/java/eval/Printer.java Printer.repr
fn repr_with(value: Value<'_>, out: &mut String, on_label: &dyn Fn(&Label, &mut String)) {
    if let Some(label) = value.downcast_ref::<LabelValue>() {
        on_label(&label.0, out);
    } else if let Some(list) = ListRef::from_value(value) {
        out.push('[');
        for (i, x) in list.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(x, out, on_label);
        }
        out.push(']');
    } else if let Some(tuple) = TupleRef::from_value(value) {
        out.push('(');
        for (i, x) in tuple.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(x, out, on_label);
        }
        if tuple.len() == 1 {
            out.push(',');
        }
        out.push(')');
    } else if let Some(dict) = DictRef::from_value(value) {
        out.push('{');
        for (i, (k, v)) in dict.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(k, out, on_label);
            out.push_str(": ");
            repr_with(v, out, on_label);
        }
        out.push('}');
    } else {
        out.push_str(&value.to_repr());
    }
}
